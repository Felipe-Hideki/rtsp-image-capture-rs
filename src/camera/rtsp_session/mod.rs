pub mod utils;

use std::time::{Duration, Instant};

use futures::StreamExt;
use openh264::formats::YUVSource;
use retina::{
    client::{
        InitialSequenceNumberPolicy, InitialTimestampPolicy, PlayOptions, Session, SessionOptions,
        SetupOptions, TcpTransportOptions, Transport,
    },
    codec::{CodecItem, VideoFrame},
};
use tokio::{sync, task::JoinHandle};
use url::Url;

use crate::decoders::{DecoderError, H264RGBDecoder, ImageDecoder};

// TODO: Maybe I should split these into different sectors
#[derive(Debug)]
pub enum SessionError {
    UrlParseError,
    UnsetParameter(String),

    ServerDropped,
    BrokenPipeline,
    UnableToSubscribe(String),
    DecodingError(DecoderError),

    OldFrame,
}

pub struct SessionInstance {
    data_req_tx: RequesterTx<Result<Vec<u8>, SessionError>>,
}

impl SessionInstance {
    fn new(data_req_tx: RequesterTx<Result<Vec<u8>, SessionError>>) -> Self {
        Self { data_req_tx }
    }

    pub async fn request_image(&self) -> Result<Vec<u8>, SessionError> {
        let (req_tx, req_rx) = sync::oneshot::channel();
        self.data_req_tx
            .send(req_tx)
            .await
            .map_err(|_| SessionError::BrokenPipeline)?;

        req_rx.await.map_err(|_| SessionError::ServerDropped)?
    }
}

type RequesterTx<T> = sync::mpsc::Sender<sync::oneshot::Sender<T>>;
type RequesterRx<T> = sync::mpsc::Receiver<sync::oneshot::Sender<T>>;

pub struct SessionInstanceManager {
    subscriber_request_tx: RequesterTx<Option<SessionInstance>>,
    task_handle: JoinHandle<()>,
}

impl SessionInstanceManager {
    fn new(
        subscriber_request_tx: RequesterTx<Option<SessionInstance>>,
        task_handle: JoinHandle<()>,
    ) -> Self {
        Self {
            subscriber_request_tx,
            task_handle,
        }
    }

    pub async fn request_instance(&mut self) -> Result<SessionInstance, SessionError> {
        let (inp, out) = sync::oneshot::channel();
        self.subscriber_request_tx
            .send(inp)
            .await
            .map_err(|_| SessionError::BrokenPipeline)?;

        out.await
            .map_err(|_| SessionError::ServerDropped)?
            .ok_or(SessionError::UnableToSubscribe(String::new())) // TODO: Change from
                                                                   // Option to Result as this
                                                                   // allows a feedback return
    }

    pub fn close(self) {}
}

impl Drop for SessionInstanceManager {
    fn drop(&mut self) {
        self.task_handle.abort();
    }
}

struct FrameHolder {
    inner_h264: Vec<u8>,
    ts: Instant,
    decoded_frame: Option<Vec<u8>>,
}

impl FrameHolder {
    fn new(h264: Vec<u8>) -> Self {
        Self {
            inner_h264: h264,
            ts: Instant::now(),
            decoded_frame: None,
        }
    }

    fn decode(&mut self, decoder: &mut dyn ImageDecoder) -> Result<&[u8], DecoderError> {
        if self.decoded_frame.is_some() {
            return self
                .decoded_frame
                .as_ref()
                .map(|v| v.as_slice())
                .ok_or(DecoderError::NoImageDecoded);
        }
        let decoded = decoder.decode(self.inner_h264.to_vec())?;

        self.decoded_frame = Some(decoded);
        self.decoded_frame
            .as_ref()
            .map(|v| v.as_slice())
            .ok_or(DecoderError::NoImageDecoded)
    }

    fn empty() -> Self {
        Self {
            inner_h264: vec![],
            ts: Instant::now(),
            decoded_frame: Some(vec![]),
        }
    }

    fn is_empty(&self) -> bool {
        if let Some(v) = self.decoded_frame.as_ref() {
            v.is_empty()
        } else {
            false
        }
    }

    fn elapsed(&self) -> Duration {
        Instant::now().duration_since(self.ts)
    }
}

pub struct SessionWrapper {
    camera_url: Url,
    frame_holder: FrameHolder,
    decoder: Box<dyn ImageDecoder + Sync + Send>,
}

impl SessionWrapper {
    pub fn new(camera_url: Url, decoder: Box<dyn ImageDecoder + Sync + Send>) -> Self {
        Self {
            camera_url,
            frame_holder: FrameHolder::empty(),
            decoder,
        }
    }

    pub async fn start(self) -> SessionInstanceManager {
        let (subscriber_requester_tx, subscriber_requester_rx) = sync::mpsc::channel(24);
        let handle = tokio::spawn(self.session_loop(subscriber_requester_rx));
        SessionInstanceManager {
            subscriber_request_tx: subscriber_requester_tx,
            task_handle: handle,
        }
    }

    fn is_iframe(f: &VideoFrame) -> bool {
        f.is_random_access_point()
    }

    async fn session_loop(mut self, mut data_requester_rx: RequesterRx<Option<SessionInstance>>) {
        let mut session = Session::describe(self.camera_url, SessionOptions::default())
            .await
            .expect("Failed to create Session");

        let video_stream = session
            .streams()
            .iter()
            .position(|s| s.media() == "video")
            .expect("No video stream available");

        println!("Setting up session");
        session
            .setup(
                video_stream,
                SetupOptions::default().transport(Transport::Tcp(TcpTransportOptions::default())),
            )
            .await
            .expect("Failed to setup session");

        println!("Playing and demuxing session");
        println!("Using stream {:?}", session.streams()[video_stream]);

        let mut session = session
            .play(
                PlayOptions::default()
                    .initial_seq(InitialSequenceNumberPolicy::Respect)
                    .initial_timestamp(InitialTimestampPolicy::Require),
            )
            .await
            .expect("Failed to start session")
            .demuxed()
            .expect("Couldn't demux session");

        type DataRequester = sync::oneshot::Sender<Result<Vec<u8>, SessionError>>;

        let (data_req_tx, mut data_req_rx) = sync::mpsc::channel::<DataRequester>(32);
        loop {
            tokio::select! {
                Some(sender) = data_req_rx.recv(), if !self.frame_holder.is_empty() => {
                    if self.frame_holder.elapsed() > Duration::from_millis(500) {
                        self.frame_holder = FrameHolder::empty();
                        sender.send(Err(SessionError::OldFrame));
                        continue;
                    }

                    sender.send(self.frame_holder.decode(&mut *self.decoder).map(|v| v.to_vec()).map_err(|e| SessionError::DecodingError(e)));

                },
                Some(req) = data_requester_rx.recv() => {
                    match req.send(Some(SessionInstance::new(data_req_tx.clone()))) {
                        Ok(_) => {},
                        Err(_) => {
                            println!("Failed to send data back")
                        }
                    }
                },
                Some(Ok(packet)) = session.next() => {
                    match packet {
                        CodecItem::VideoFrame(f) => {
                            if !Self::is_iframe(&f) {
                                continue;
                            }
                            self.frame_holder = FrameHolder::new(f.into_data())
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

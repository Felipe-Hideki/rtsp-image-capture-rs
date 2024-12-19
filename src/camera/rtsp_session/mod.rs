pub mod utils;

use std::{
    fmt::Debug,
    time::{Duration, Instant},
};

use futures::StreamExt;
use retina::{
    client::{
        Demuxed, InitialSequenceNumberPolicy, InitialTimestampPolicy, PlayOptions, Session,
        SessionOptions, SetupOptions, TcpTransportOptions, Transport,
    },
    codec::{CodecItem, VideoFrame},
    Error,
};
use tokio::{sync, task::JoinHandle};
use url::Url;

use crate::decoders::{DecoderError, ImageDecoder};

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
    FailedToDescribeSession(Error),
    NoVideoStreamFound,
    FailedToSetupStream(Error),
    FailedToPlayStream(Error),
    FailedToDemuxStream(Error),
}

type FrameRequester = sync::mpsc::Sender<FrameRequest>;
pub struct SessionInstance {
    data_req_tx: FrameRequester,
}

impl SessionInstance {
    fn new(data_req_tx: FrameRequester) -> Self {
        Self { data_req_tx }
    }

    pub async fn request_image(
        &self,
        mut req: FrameRequest,
    ) -> Result<FrameResponse, SessionError> {
        let (req_tx, req_rx) = sync::oneshot::channel();
        req.with_tx(req_tx);
        self.data_req_tx
            .send(req)
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

#[derive(Clone)]
struct FrameHolder {
    raw_frames: Vec<Vec<u8>>,
    ts: Instant,
    decoded_frames: Vec<Vec<u8>>,
}

impl FrameHolder {
    fn new() -> Self {
        Self {
            raw_frames: Vec::new(),
            ts: Instant::now(),
            decoded_frames: Vec::new(),
        }
    }

    fn decode(
        &mut self,
        decoder: &mut dyn ImageDecoder,
        index: usize,
    ) -> Result<&[u8], DecoderError> {
        if index >= self.raw_frames.len() {
            return Err(DecoderError::IndexOutOfBounds);
        }
        if self.decoded_frames.len() < index {
            self.decode(decoder, index - 1)?;
        }
        if index < self.decoded_frames.len() {
            return self
                .decoded_frames
                .get(index)
                .map(|v| v.as_slice())
                .ok_or(DecoderError::NoImageDecoded);
        }
        let decoded = decoder.decode(&self.raw_frames[index])?;

        self.decoded_frames.push(decoded.to_vec());
        self.decoded_frames
            .get(index)
            .map(|v| v.as_slice())
            .ok_or(DecoderError::NoImageDecoded)
    }

    fn set_iframe(&mut self, iframe: Vec<u8>) {
        self.raw_frames.clear();
        self.decoded_frames.clear();
        self.ts = Instant::now();

        self.raw_frames.push(iframe);
    }

    fn add_image(&mut self, data: Vec<u8>) {
        self.raw_frames.push(data)
    }

    fn drain(&mut self) {
        self.raw_frames.clear();
        self.decoded_frames.clear();
        self.ts = Instant::now()
    }

    fn is_empty(&self) -> bool {
        self.raw_frames.is_empty()
    }

    fn raw_len(&self) -> usize {
        self.raw_frames.len()
    }

    fn elapsed(&self) -> Duration {
        Instant::now().duration_since(self.ts)
    }
    fn get_ts(&self) -> Instant {
        self.ts
    }
}

type ReturnTx = sync::oneshot::Sender<Result<FrameResponse, SessionError>>;

pub struct FrameRequest {
    return_rx: Option<ReturnTx>,
    buf_index: usize,
}

impl FrameRequest {
    pub fn new(index: usize) -> Self {
        Self {
            return_rx: None,
            buf_index: index,
        }
    }

    fn with_tx(&mut self, rx: ReturnTx) {
        self.return_rx = Some(rx)
    }
}

impl Debug for FrameRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FrameRequest {\n")?;
        f.write_str(format!("    buf_index: {}\n", self.buf_index).as_str())?;
        f.write_str("}\n")?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct FrameResponse {
    frame: Vec<u8>,
    i_frame_ts: Instant,
}

pub struct SessionConfig {
    pub buf_size: usize,
    pub frame_lifetime: Duration,
}

pub struct SessionWrapper {
    camera_url: Url,
    frame_holder: FrameHolder,
    decoder: Box<dyn ImageDecoder + Sync + Send>,
    cfg: SessionConfig,
}

impl SessionWrapper {
    pub fn new(
        camera_url: Url,
        decoder: Box<dyn ImageDecoder + Sync + Send>,
        cfg: SessionConfig,
    ) -> Self {
        Self {
            camera_url,
            frame_holder: FrameHolder::new(),
            decoder,
            cfg,
        }
    }

    pub async fn start(self) -> SessionInstanceManager {
        let (subscriber_requester_tx, subscriber_requester_rx) = sync::mpsc::channel(24);
        let handle = tokio::spawn(self.session_loop(subscriber_requester_rx));
        SessionInstanceManager::new(subscriber_requester_tx, handle)
    }

    async fn start_session(&self) -> Result<Demuxed, SessionError> {
        let mut session = Session::describe(self.camera_url.clone(), SessionOptions::default())
            .await
            .map_err(|e| SessionError::FailedToDescribeSession(e))?;

        let video_stream = session
            .streams()
            .iter()
            .position(|s| s.media() == "video")
            .ok_or(SessionError::NoVideoStreamFound)?;

        session
            .setup(
                video_stream,
                SetupOptions::default().transport(Transport::Tcp(TcpTransportOptions::default())),
            )
            .await
            .map_err(|e| SessionError::FailedToSetupStream(e))?;

        session
            .play(
                PlayOptions::default()
                    .initial_seq(InitialSequenceNumberPolicy::Respect)
                    .initial_timestamp(InitialTimestampPolicy::Require),
            )
            .await
            .map_err(|e| SessionError::FailedToPlayStream(e))?
            .demuxed()
            .map_err(|e| SessionError::FailedToDemuxStream(e))
    }

    async fn session_loop(mut self, mut data_requester_rx: RequesterRx<Option<SessionInstance>>) {
        let mut session = self
            .start_session()
            .await
            .expect("Failed to start session stream");

        let (data_req_tx, mut data_req_rx) = sync::mpsc::channel::<FrameRequest>(32);
        loop {
            tokio::select! {
                Some(mut req) = data_req_rx.recv(), if !self.frame_holder.is_empty() => {
                    let sender = match req.return_rx.take() {
                        Some(s) => s,
                        None => continue
                    };
                    if self.frame_holder.elapsed() > self.cfg.frame_lifetime {
                        self.frame_holder.drain();
                        match sender.send(Err(SessionError::OldFrame)) {
                            Ok(_) => {},
                            Err(e) => {
                                println!("Channel was closed by requester: {:?}", e)
                            }
                        }
                        continue;
                    }
                    let f = self.frame_holder.decode(&mut *self.decoder, req.buf_index)
                        .map_or_else(|e| Err(SessionError::DecodingError(e)), |v| Ok(v.to_vec()));

                    let resp = f.map(|x| FrameResponse {frame: x, i_frame_ts: self.frame_holder.get_ts()});
                    match sender.send(resp) {
                        Ok(_) => {},
                        Err(e) => {
                            println!("Channel was closed by requester: {:?}", e)
                        }
                    }

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
                            if f.is_random_access_point() {
                                self.frame_holder.set_iframe(f.into_data());
                                continue;
                            }
                            if self.frame_holder.raw_len() >= self.cfg.buf_size {
                                continue;
                            }
                            self.frame_holder.add_image(f.into_data());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

use std::{
    env::args,
    time::{Duration, Instant},
};

mod camera;
mod decoders;

use camera::{
    onvif::{services, OnvifHelper},
    rtsp_session::{SessionError, SessionWrapper},
};
use decoders::{AVCCDecoder, Chain, H264RGBDecoder};

#[tokio::main]
async fn main() {
    let mut args = args().skip(1);
    let ip = args.next().expect("Ip not found");
    let (user, password) = (
        args.next().expect("Credentials not inputted"),
        args.next().expect("Credentials not inputted"),
    );

    let mut onvif_client = OnvifHelper::new(&ip)
        .expect("Failed to create OnvifHelper")
        .with_credentials(&user, &password);

    let media_cli = onvif_client
        .get_service::<services::MediaClient>()
        .await
        .expect("Couldn't fetch media client");

    let (res, media_cli) = {
        let mut profiles = media_cli
            .get_profiles()
            .await
            .expect("Failed to get profiles");

        if profiles.len() == 0 {
            panic!("Empty profiles list");
        }

        let first_profile = profiles.remove(0);
        let token = first_profile.token;
        if let Some(conf) = first_profile.video_encoder_configuration {
            (
                (
                    conf.resolution.width as usize,
                    conf.resolution.height as usize,
                ),
                media_cli.with_token(token),
            )
        } else {
            panic!("Video encoder configuration is missing");
        }
    };

    println!("Fetching stream url...");
    let stream_url = media_cli
        .get_stream_uri()
        .await
        .expect("Stream url unavailable");

    let decoder = Box::new(
        AVCCDecoder::new()
            .chain(H264RGBDecoder::new(true, res).expect("Failed to create h264 decoder")),
    );

    let mut session = SessionWrapper::new(stream_url, decoder).start().await;
    let instance = session
        .request_instance()
        .await
        .expect("Couldn't fetch session instance");

    loop {
        let b = Instant::now();
        let _abc = match instance.request_image().await {
            Ok(f) => Ok(f),
            Err(SessionError::OldFrame) => {
                media_cli
                    .sync_iframe()
                    .await
                    .expect("Failed to sync iframes");
                Err(SessionError::OldFrame)
            }
            Err(e) => {
                println!("Unexpected Error => {:?}", e);
                Err(e)
            }
        };
        println!(
            "Captured -> {:?} in {} ms",
            _abc.as_ref().err(),
            Instant::now().duration_since(b).as_millis()
        );
        if !_abc.is_err() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

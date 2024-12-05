use std::{
    env::args,
    time::{Duration, SystemTime},
};

mod camera;
mod decoders;

use camera::onvif::{services, OnvifHelper};
use decoders::H264Decoder;
use retina::{
    self,
    client::{
        InitialSequenceNumberPolicy, InitialTimestampPolicy, PlayOptions, Session, SessionOptions,
        SetupOptions, TcpTransportOptions, Transport,
    },
    codec::CodecItem,
};
use url::Url;

use futures::StreamExt;

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
        .expect("Couldn't fetch media client")
        .with_first_profile()
        .await
        .expect("Failed to fetch profile token");

    let mut session = Session::describe(
        Url::parse(
            &("rtsp://".to_string()
                + &ip
                + &format!(":554/user={}&password={}&channel=2", user, password)),
        )
        .expect("Failed to parse url"),
        SessionOptions::default(),
    )
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

    let mut last_saved = SystemTime::now();

    let mut decoder = H264Decoder::new(true).unwrap();

    let mut i_frames_indices = Vec::new();
    let mut i = 0;
    let mut frame_buf = vec![0u8; 1920 * 1080 * 3];

    println!("loop start");
    let mut sent = false;
    loop {
        let pkt = session.next().await;
        if pkt.is_none() {
            eprintln!("{:?}", pkt);
            continue;
        }
        match pkt.unwrap().unwrap() {
            CodecItem::VideoFrame(f) => {
                i += 1;
                println!(
                    "Frame len {} -> disposable? {}",
                    f.data().len(),
                    f.is_random_access_point()
                );
                if f.is_random_access_point() {
                    i_frames_indices.push(i);
                    println!("I-Frame indices -> {:?}", i_frames_indices);
                }
                if last_saved.elapsed().unwrap() > Duration::from_millis(5000) {
                    if !f.is_random_access_point() {
                        if !sent {
                            media_cli.sync_iframe().await.unwrap();
                            sent = true
                        }
                    } else {
                        let annex_b = decoder.decode_to_annex_b(f.data()).unwrap().to_vec();
                        let yuv_image = decoder.decode(&annex_b).unwrap().unwrap();
                        yuv_image.write_rgb8(&mut frame_buf);
                        let rgb_image = turbojpeg::Image {
                            pixels: &frame_buf as &[u8],
                            width: 1920,
                            height: 1080,
                            pitch: 1920 * 3,
                            format: turbojpeg::PixelFormat::RGB,
                        };
                        let compressed =
                            turbojpeg::compress(rgb_image, 90, turbojpeg::Subsamp::Sub2x2)
                                .expect("Failed to compress image");
                        std::fs::write("t.jpg", compressed).expect("Failed to write image");
                        last_saved = SystemTime::now();
                        sent = false;
                    }
                }
                //frames.push(annex_b);
            }
            CodecItem::Rtcp(rtcp) => {
                if let (Some(t), Some(Ok(Some(sr)))) = (
                    rtcp.rtp_timestamp(),
                    rtcp.pkts()
                        .next()
                        .map(retina::rtcp::PacketRef::as_sender_report),
                ) {
                    println!("{}: SR ts={}", t, sr.ntp_timestamp());
                }
            }
            _ => continue,
        }
    }
}

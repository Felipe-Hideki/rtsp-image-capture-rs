use std::{
    env::args,
    str::FromStr,
    time::{Duration, SystemTime},
};

mod decoders;

use base64::prelude::*;
use decoders::H264Decoder;
use onvif::{
    schema::{self, onvif::ReferenceToken},
    soap::client::Credentials,
};
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

fn extract_pps_sps(buf: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let buf_as_str = String::from_utf8_lossy(buf).to_string();
    let (pps, sps) = buf_as_str
        .lines()
        .filter(|x| x.starts_with("a=fmtp:96"))
        .collect::<Vec<&str>>()[0]
        .split(';')
        .last()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .split_once(',')
        .unwrap();

    (
        BASE64_STANDARD.decode(pps).unwrap(),
        BASE64_STANDARD.decode(sps).unwrap(),
    )
}

#[tokio::main]
async fn main() {
    let mut args = args().skip(1);
    let ip = args.next().expect("Ip not found");
    let (user, password) = (
        args.next().expect("Credentials not inputted"),
        args.next().expect("Credentials not inputted"),
    );
    let onvif_url = Url::from_str(&("http://".to_string() + &ip + ":8899")).unwrap();
    let mgmt_url = onvif_url.join("/onvif/device_service").unwrap();
    let creds = Credentials {
        username: user.to_string(),
        password: password.to_string(),
    };

    let mgmt_client = onvif::soap::client::ClientBuilder::new(&mgmt_url)
        .credentials(Some(creds.clone()))
        .build();

    let services = onvif::schema::devicemgmt::get_capabilities(&mgmt_client, &Default::default())
        .await
        .unwrap();
    let media_url: Url = Url::from_str(&services.capabilities.media[0].x_addr).unwrap();
    let media_client = onvif::soap::client::ClientBuilder::new(&media_url)
        .credentials(Some(creds))
        .build();

    let mut profiles = schema::media::get_profiles(&media_client, &Default::default())
        .await
        .unwrap()
        .profiles;

    let profile = profiles.remove(0);

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

    let (sps, pps) = extract_pps_sps(session.sdp());
    println!("{:?}, {:?}", pps, sps);

    let mut header = vec![0x00, 0x00, 0x00, 0x01u8];
    header.extend_from_slice(&sps);
    header.extend_from_slice(&[0x00, 0x00, 0x00, 0x01u8]);
    header.extend_from_slice(&pps);
    println!("Headers {:?}", header);
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
    decoder.decode(&header).unwrap();
    drop(header);

    let mut i_frames_indices = Vec::new();
    let mut i = 0;

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
                if last_saved.elapsed().unwrap() > Duration::from_millis(200) {
                    if !f.is_random_access_point() {
                        if !sent {
                            let token = ReferenceToken::from_str(&profile.token.to_string());
                            let cli_clone = media_client.clone();
                            tokio::spawn(async move {
                                let sync_req = {
                                    let mut sync_req =
                                        schema::media::SetSynchronizationPoint::default();
                                    sync_req.profile_token = token.unwrap_or_default();
                                    sync_req
                                };
                                schema::media::set_synchronization_point(&cli_clone, &sync_req)
                                    .await
                            });
                            sent = true
                        }
                    } else {
                        //let rgb_image = Image {
                        //    pixels: &frame_buf as &[u8],
                        //    width: 1920,
                        //    height: 1080,
                        //    pitch: 1920 * 3,
                        //    format: turbojpeg::PixelFormat::RGB,
                        //};
                        //let compressed = turbojpeg::compress(rgb_image, 90, turbojpeg::Subsamp::Sub2x2)
                        //    .expect("Failed to compress image");
                        //std::fs::write(save_path, compressed).expect("Failed to write image");
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

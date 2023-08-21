use anyhow::Result;
use log::{info, trace, warn};
use openh264::decoder::{DecodedYUV, Decoder, DecoderConfig};
use rtsp_client::{Methods, Session};
use std::fs::File;
use std::io::prelude::*;
use std::net::Shutdown;
use std::path::Path;
use tokio::net::UdpSocket;
//------------------SDL2
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::Canvas;

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let mut rtsp = Session::new("192.168.86.112:554".to_string())?;

    let response = rtsp.send(Methods::Options).await?;
    info!("OPTIONS: \n{response}");

    let response = rtsp.send(Methods::Describe).await?;
    info!("DESCRIBE: \n{response}");

    let response = rtsp.send(Methods::Setup).await?;
    info!("SETUP: \n{response}");

    let response = rtsp.send(Methods::Play).await?;
    info!("PLAY: \n{response}");

    if (&response).contains("200 OK") {
        // Bind to my client UDP port which is provided in DESCRIBE method
        // in the 'Transport' header
        let udp_stream = UdpSocket::bind("0.0.0.0:4588").await?;

        // Connect to the RTP camera server using IP and port
        // provided in SETUP response
        // In the RTP specs, the RTCP server should be
        // port 6601 and will always need to be
        // a different port
        udp_stream.connect("192.168.86.112:6600").await?;

        // Setup OpenH264 decoder
        let decoder_config = DecoderConfig::new();
        decoder_config.debug(true);
        let mut decoder = Decoder::with_config(decoder_config)?;

        // ----------------- NOTE
        // Most implementations will break up IDR frames
        // into fragments (e.g. FU-A)
        // see section 5.8 of RFC 6184

        // PAYLOAD starts at byte 14
        // which in 0 index array = 13
        // UNLESS this is a fragment (e.g. FU-A)
        // in which case it's byte 15
        // as FU-A has extra byte for header

        // Start prefix code (3 or 4 bytes)
        // For beginning of entire stream or SPS/PPS nal units -> 0x00 0x00 x00 0x01
        // All other nal units use -> 0x00 0x00 0x01

        // Byte index where NAL unit starts in RTP packet
        // This is also where the NAL header is which is 1 byte
        let mut buf_rtp = [0u8; 2048];
        let mut buf_temp: Vec<u8> = Vec::new();
        let mut buf_sps: Vec<u8> = Vec::new();
        let mut buf_fragments: Vec<u8> = Vec::new();
        let mut buf_all: Vec<u8> = Vec::new();

        let mut is_sps_found = false;
        let mut is_start_decoding = false;
        let mut is_fragment_start = false;
        let mut is_fragment_end = false;

        const NAL_UNIT_START: usize = 12;

        // NOTE: Display decoded images with SDL2
        let sdl_context = sdl2::init().expect("Error sdl2 init");
        let video_subsystem = sdl_context.video().expect("Error sld2 video subsystem");

        let window = video_subsystem
            .window("IP Camera Video", 640, 352)
            .position_centered()
            .opengl()
            .build()?;

        let mut canvas = window.into_canvas().build()?;
        let texture_creator = canvas.texture_creator();

        // TODO: Figure out how to move this into loop
        // so as not to have to apply static definition
        let mut texture = texture_creator.create_texture_static(PixelFormatEnum::IYUV, 640, 352)?;
        let mut event_pump = sdl_context.event_pump().expect("Error sld2 event");

        loop {
            for event in event_pump.poll_iter() {
                match event {
                    Event::Quit { .. }
                    | Event::KeyDown {
                        keycode: Some(Keycode::Escape),
                        ..
                    } => break,
                    _ => {}
                }
            }

            let len = udp_stream.recv(&mut buf_rtp).await?;
            // Byte 12 is NAL unit header (because of 0 index)
            // Previous bytes are RTP header
            // 13th byte is NAL header which in 0 index array = 12
            let header_nal = &buf_rtp[NAL_UNIT_START];

            info!("{} bytes received", len);
            info!("-----------\n{:08b}", header_nal);

            // Check if this is an SPS packet
            // NAL header byte -> 01100111
            if *header_nal == 103u8 {
                // if is_start_decoding && num_sps == 7 {
                //     save_file(buf_all.as_slice());
                //     break;
                // }

                trace!("Sequence started! --------------------------------------");

                is_sps_found = true;
                // num_sps += 1;

                // Store entire SPS NAL unit including header for later
                buf_sps.extend_from_slice(&[0u8, 0u8, 0u8, 1u8]);
                buf_sps.extend_from_slice(&buf_rtp[NAL_UNIT_START..len]);
            }
            // Check if this is an PPS packet
            // NAL header byte -> 01101000
            else if *header_nal == 104u8 {
                info!("PPS packet ----- ");

                if is_sps_found {
                    is_start_decoding = true;

                    buf_temp.extend_from_slice(buf_sps.as_slice());
                    buf_temp.extend_from_slice(&[0u8, 0u8, 0u8, 1u8]);
                    buf_temp.extend_from_slice(&buf_rtp[NAL_UNIT_START..len]);
                    buf_sps.clear();
                }
            }
            // Check if this is an SEI packet
            // NAL header byte -> 00000110
            else if *header_nal == 6u8 {
                info!("SEI packet ----- ");

                buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
                buf_temp.extend_from_slice(&buf_rtp[NAL_UNIT_START..len]);
            }
            // Check for fragment (FU-A)
            // NAL header byte -> 01111100
            else if *header_nal == 124u8 {
                info!("Fragment started!! ----- ");

                is_fragment_start = true;
                //  +---------------+
                // |0|1|2|3|4|5|6|7|
                // +-+-+-+-+-+-+-+-+
                // |S|E|R|  Type   |
                // +---------------+
                // S = Start
                // E = End

                // Check fragment header which is byte
                // after NAL header
                let header_frag = &buf_rtp[13];
                info!("Fragment header -- {:08b}", header_frag);

                // Is this fragment START?
                // if *header_frag & 0b10000000 == 128u8 {
                //     // Do Nothing
                // }

                // Note: Do I reassemble fragments with just it's payload
                // or the entire NAL unit??

                // Or fragment END?
                if *header_frag & 0b01000000 == 64u8 {
                    trace!("Fragment ended!! ----- ");

                    // Need to know when fragments end to combine and send to decoder
                    is_fragment_end = true;

                    buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
                    // Need to swap outside nal header to inside payload type
                    // as after combining packet it's not a fragment anymore
                    // TODO: Need to get this from fragment header type instead of hard coding
                    buf_temp.extend_from_slice(&[101u8]); // 01100101 = 101u8
                    buf_temp.extend_from_slice(buf_fragments.as_slice());
                    buf_temp.extend_from_slice(&buf_rtp[14..len]);
                    buf_fragments.clear();
                } else {
                    // Append fragment payload EXCLUDING ALL HEADERS
                    buf_fragments.extend_from_slice(&buf_rtp[14..len]);
                }
            } else {
                info!("Slice packet ----- ");

                is_sps_found = false;

                buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
                buf_temp.extend_from_slice(&buf_rtp[NAL_UNIT_START..len]);
            }

            // Skip packets until we first get SPS AND PPS pair
            // TODO: Should I clear this out after reading header pair??
            if is_start_decoding && buf_temp.len() > 0 {
                if is_fragment_start {
                    if is_fragment_end {
                        is_fragment_start = false;
                        is_fragment_end = false;
                    } else {
                        continue;
                    }
                }

                // all current packets data
                buf_all.extend_from_slice(buf_temp.as_slice());

                // DECODE
                // Idea is to store all packets depending on types in buf_temp
                // SPS/PPS     = 2 packets
                // Fragment    = 1 packet COMBINED
                // Slice       = 1 packet
                info!("//////////////////////////////////////////");
                info!("Decoding packet size: {:?}", buf_temp.len());

                let maybe_some_yuv = decoder.decode(buf_temp.as_slice());
                match maybe_some_yuv {
                    Ok(some_yuv) => match some_yuv {
                        Some(yuv) => {
                            info!("Decoded YUV!");

                            let (y_size, u_size, v_size) = yuv.strides_yuv();
                            texture.update_yuv(
                                None,
                                yuv.y_with_stride(),
                                y_size,
                                yuv.u_with_stride(),
                                u_size,
                                yuv.v_with_stride(),
                                v_size,
                            );

                            canvas.clear();
                            canvas
                                .copy(&texture, None, None)
                                .expect("Error copying texture");
                            canvas.present();
                        }
                        None => info!("Unable to decode to YUV"),
                    },
                    // Errors from OpenH264-rs have been useless as they are mostly
                    // native errors passed from C implementation and then propogated
                    // to Rust as a single i64 code and I couldn't find anywhere to
                    // convert this i64 code to it's description...
                    // Instead, I had to use ffprobe after saving out a large raw
                    // stream of decoded packets to file
                    Err(e) => warn!("Error: {e}"),
                }
            }

            buf_temp.clear();
        }
    }

    info!("Stopping RTSP: {}", rtsp.stop());
    Ok(())
}

fn save_file(buffer: &[u8]) {
    let path = Path::new("test_file.h264");
    let display = path.display();

    // Open a file in write-only mode, returns `io::Result<File>`
    let mut file = match File::create(&path) {
        Err(why) => panic!("couldn't create {}: {}", display, why),
        Ok(file) => file,
    };

    // Write the `LOREM_IPSUM` string to `file`, returns `io::Result<()>`
    match file.write_all(buffer) {
        Err(why) => panic!("couldn't write to {}: {}", display, why),
        Ok(_) => info!("successfully wrote to {}", display),
    }
}

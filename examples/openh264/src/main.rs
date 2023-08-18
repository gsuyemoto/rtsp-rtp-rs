use anyhow::Result;
use ctrlc;
use openh264::{decoder::Decoder, decoder::DecoderConfig};
use rtsp_client::{Methods, Session};
use std::fs::File;
use std::io::prelude::*;
use std::io::Cursor;
use std::path::Path;
use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> Result<()> {
    let mut rtsp = Session::new("192.168.86.138:554".to_string())?;

    let response = rtsp.send(Methods::Options).await?;
    println!("OPTIONS: \n{response}");

    let response = rtsp.send(Methods::Describe).await?;
    println!("DESCRIBE: \n{response}");

    let response = rtsp.send(Methods::Setup).await?;
    println!("SETUP: \n{response}");

    let response = rtsp.send(Methods::Play).await?;
    println!("PLAY: \n{response}");

    if (&response).contains("200 OK") {
        // Bind to my client UDP port which is provided in DESCRIBE method
        // in the 'Transport' header
        let udp_stream = UdpSocket::bind("0.0.0.0:4588").await?;

        // Connect to the RTP camera server using IP and port
        // provided in SETUP response
        // In the RTP specs, the RTCP server should be
        // port 6601 and will always need to be
        // a different port
        udp_stream.connect("192.168.86.138:6600").await?;

        // Setup OpenH264 decoder
        let decoder_config = DecoderConfig::new();
        decoder_config.debug(true);
        let mut decoder = Decoder::with_config(decoder_config)?;

        // Set buffer to large enough to handle RTP packets
        // in my Wireshark analysis for this camera they
        // tended be a bit more than 1024
        let mut buf_rtp = [0u8; 2048];
        let mut num_packets = 0;
        let mut sequence_started = false;

        // OpenH264 expects TWO zero bytes and then prefix
        // start code to indicate beginning of stream
        let mut buf_nal: Vec<u8> = Vec::new();
        buf_nal.push(0u8);
        buf_nal.push(0u8);

        loop {
            let len = udp_stream.recv(&mut buf_rtp).await?;
            // Byte 12 is NAL unit header
            let header_nal = &buf_rtp[12];

            println!("{} bytes received", len);
            println!("-----------\n{:08b}", header_nal);

            // Check if this is an SPS packet
            // First byte NAL type should be -> 00111
            if *header_nal & 0b00000111 == 7u8 {
                if sequence_started {
                    // Save one entire NAL sequence from SPS to SPS
                    save_file(buf_nal.as_slice());

                    // Try decode using entire sequence
                    let maybe_some_yuv = decoder.decode(buf_nal.as_slice())?;
                    match maybe_some_yuv {
                        Some(yuv) => println!("Decoded YUV!"),
                        None => println!("Unable to decode to YUV"),
                    }

                    break;
                } else {
                    println!("Sequence started! --------------------------------------");
                    sequence_started = true;
                }
            }

            num_packets += 1;

            // add start prefix code
            // before eash NAL unit
            buf_nal.push(1u8);
            buf_nal.extend_from_slice(&buf_rtp[12..len]);

            if num_packets == 1 && !sequence_started {
                // Need to start with SPS, otherwise fail
                break;
            }
        }
    }

    println!("Stopping RTSP: {}", rtsp.stop());
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
        Ok(_) => println!("successfully wrote to {}", display),
    }
}

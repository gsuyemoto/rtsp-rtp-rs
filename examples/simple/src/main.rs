use anyhow::Result;
use ctrlc;
use openh264::{decoder::Decoder, decoder::DecoderConfig};
use rtsp_client::{Methods, Session};
use std::io::Cursor;
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

        // Keep a separate buffer for the NAL units
        // which should be the payload of each
        // RTP packet. Some NAL units may not
        // contain enough info on their own and
        // may need more units, hence the buffer
        // let mut payload: Vec<u8> = Vec::new();

        // Capture X num fragments and then exit
        let mut sequence_started = false;

        let mut num_packets = 0;
        // let mut payload_buf: Vec<u8> = Vec::new();

        // Packet sequence for RTP using H264 and
        // packetization-mode=1 (non-interleaved mode)
        // Seems to go like this:
        //
        // Packet 1 - SPS (NAL Type 7) ---------------------|
        // Packet 2 - PPS (NAL Type 8)                      |
        // Packet 3 - SEI (NAL Type 6)                      |
        // Packet 4 - FU-A (NAL Type 28) Start              |-- First Packet Sequence
        // Packet 5 - FU-A (NAL Type 28)                    |
        // Packet 6 - FU-A (NAL Type 28) End                |
        // Packet 7 - Coded Slice Non-IDR (NAL Type 1)      |
        // Packet 8+ - More Coded Slices (NAL Type 1)-------|
        //
        // Packet 1 - SPS (NAL Type 7)----------------------|
        // Packet 2 - PPS (NAL Type 8)                      |
        // Packet 3 - SEI (NAL Type 6)                      |
        // Packet 4 - FU-A (NAL Type 28) Start              |-- Second Packet Sequence, etc.
        // Packet 5 - FU-A (NAL Type 28)                    |
        // Packet 6 - FU-A (NAL Type 28) End                |
        // Packet 7 - Coded Slice Non-IDR (NAL Type 1)      |
        // Packet 8+ - More Coded Slices (NAL Type 1)-------|
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
                    break;
                } else {
                    println!("Sequence started! --------------------------------------");
                    sequence_started = true;
                }
            }

            // Packetization-mode=1 found in SDP of Describe
            // non-interleaved mode
            // Packet of NAL type 28 denotes FU-A
            // NAL type is last 5 bits -> 11100
            // FU headers are 2 bytes
            if *header_nal & 0b00011100 == 28u8 {
                // Byte 13 is FU header
                let header_fu = &buf_rtp[13];
                println!("{:08b} -- {:08b}", header_nal, header_fu);

                // Add FU payload to buffer which is
                // RTP packet minus RTP header minus FU header
                // payload = packet - 14 bytes
                // payload.extend_from_slice(&buf_rtp[14..len]);
                // payload.extend_from_slice(&buf_rtp[12..len]);
                // println!("FRAGMENT packet received. Buffer length: {}", payload.len());

                // Look for an IDR fragment
                // which is detemined by NAL type in last 5 bits
                // IDR is NAL type 5 which is 101 for last 5 bits

                // FU header = 10000101 -- IDR fragment start
                // FU header = 00000101 -- IDR fragment middle
                // FU header = 01000101 -- IDR fragment end
                if *header_fu == 133u8 || *header_fu == 69u8 || (*header_fu == 5u8) {
                    // End of fragment, try to decode
                    if *header_fu == 69u8 {}
                }
            } else {
                // First 12 bytes AT LEAST are for the RTP
                // header and this header can be longer
                // depending on CC flag bit
                // header.len() == 12 + (CC * 4)
                // payload = packet - 12 bytes
                // payload.extend_from_slice(&buf_rtp[12..len]);
                // println!("Non fragment packet. Buffer length: {}", payload.len());
            }

            num_packets += 1;
            // payload_buf.extend_from_slice(&buf_rtp[12..len]);
            // println!("Payload buf size: {}", payload_buf.len());

            // if sequence_started && num_packets == 3 {
            // Attempt to decode with H264
            match decoder.decode(&buf_rtp[12..len]) {
                Ok(maybe_yuv) => match maybe_yuv {
                    Some(yuv) => println!("Decoded YUV!"),
                    None => println!("Unable to decode to YUV"),
                },
                Err(e) => eprintln!("Decoding error: {e}"),
            }

            // }

            if num_packets == 40 {
                break;
            }
        }
    }

    println!("Stopping RTSP: {}", rtsp.stop());
    Ok(())
}

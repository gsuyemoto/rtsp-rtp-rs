use anyhow::Result;
use log::{debug, info, trace};
use openh264::decoder::{DecodedYUV, Decoder};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;

pub enum Decoders {
    OpenH264,
}

pub struct Rtp {
    socket: UdpSocket,
    addr_client: SocketAddr,
    addr_server: SocketAddr,
    type_decoder: Option<Decoders>,
    decoder: Option<Decoder>,
    buf_rtp: [u8; 2048],
    buf_temp: Vec<u8>,
    buf_sps: Vec<u8>,
    buf_fragments: Vec<u8>,
    buf_all: Vec<u8>,
    is_sps_found: bool,
    is_start_decoding: bool,
    is_fragment_start: bool,
    is_fragment_end: bool,
}

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
const NAL_UNIT_START: usize = 12;

impl Rtp {
    pub async fn new(
        client_ip: Option<&str>,
        client_port: u16,
        addr_server: SocketAddr,
    ) -> Result<Self> {
        // Allow manual selection of client IP which is IP that RTP/UDP server socket will listen
        // otherwise use default of 0.0.0.0
        // client PORT is chosen normally before RTSP comm and sent to server during 'SETUP' command
        // server responds with it's server PORT to send RTP
        let addr_client = match client_ip {
            Some(ip) => SocketAddr::new(IpAddr::V4(ip.parse()?), client_port),
            None => format!("0.0.0.0:{client_port}").parse()?,
        };

        let socket = UdpSocket::bind(addr_client).await?;

        let result = Rtp {
            socket,
            addr_client,
            addr_server,
            type_decoder: None,
            decoder: None,
            buf_rtp: [0u8; 2048],
            buf_temp: Vec::new(),
            buf_sps: Vec::new(),
            buf_fragments: Vec::new(),
            buf_all: Vec::new(),
            is_sps_found: false,
            is_start_decoding: false,
            is_fragment_start: false,
            is_fragment_end: false,
        };

        Ok(result)
    }

    pub async fn connect(&mut self, decoder: Decoders) -> Result<()> {
        match decoder {
            Decoders::OpenH264 => {
                let openh264_decoder = Decoder::new()?;
                self.decoder = Some(openh264_decoder);
            }
        }

        self.type_decoder = Some(decoder);
        // Connect to the RTP camera server using IP and port
        // provided in SETUP response
        // In the RTP specs, the RTCP server should be
        // port 6601 and will always need to be
        // a different port
        self.socket.connect(self.addr_server).await?;

        Ok(())
    }

    pub async fn save_file(&self) {
        let path = Path::new("video.h264");
        let display = path.display();

        // Open a file in write-only mode, returns `io::Result<File>`
        let mut file = match File::create(&path).await {
            Err(why) => panic!("couldn't create {}: {}", display, why),
            Ok(file) => file,
        };

        match file.write_all(&self.buf_all).await {
            Err(why) => panic!("couldn't write to {}: {}", display, why),
            Ok(_) => info!("successfully wrote to {}", display),
        }
    }

    pub async fn get_rtp(&mut self) -> Result<()> {
        let len = self.socket.recv(&mut self.buf_rtp).await?;

        // Get first 16 BITS of RTP packet which is part of header (RFC 6184)
        let rtp_header_pt1 = &self.buf_rtp[0];
        let rtp_header_pt2 = &self.buf_rtp[1];
        trace!(
            "RTP Header ------->>> {:08b}{:08b}",
            rtp_header_pt1,
            rtp_header_pt2
        );

        // NAL Unit Header (1st byte of NAL unit)
        // +---------------+
        // |0|1|2|3|4|5|6|7|
        // +-+-+-+-+-+-+-+-+
        // |F|NRI|  Type   |
        // +---------------+

        // BYTE 12 is NAL unit header (because of 0 index)
        let nal_header = &self.buf_rtp[NAL_UNIT_START];

        // Get the NAL unit header TYPE (last 8 BITS)
        // Use mask 00011111 = decimal 31
        let nal_header_type = nal_header & 31;

        trace!("{} bytes received", len);
        trace!("-----------\n{:08b}", nal_header);
        trace!(
            "NAL HEADER TYPE: ---------->>> {}:{}",
            nal_header_type,
            get_nal_type(nal_header_type)
        );

        trace!("NAL HEADER ---->> {:08b}", nal_header);

        // Check if this is an SPS packet
        // NAL header byte -> 01100111
        if nal_header_type == 7u8 {
            trace!("Sequence started! --------------------------------------");

            self.is_sps_found = true;
            self.buf_sps.extend_from_slice(&[0u8, 0u8, 0u8, 1u8]);
            self.buf_sps
                .extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
        }
        // Check if this is an PPS packet
        else if nal_header_type == 8u8 {
            debug!("PPS packet ----- ");

            if self.is_sps_found {
                self.is_start_decoding = true;

                self.buf_temp.extend_from_slice(self.buf_sps.as_slice());
                self.buf_temp.extend_from_slice(&[0u8, 0u8, 0u8, 1u8]);
                self.buf_temp
                    .extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
                self.buf_sps.clear();
            }
        }
        // Check if this is an SEI packet
        else if nal_header_type == 6u8 {
            debug!("SEI packet ----- ");

            self.buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
            self.buf_temp
                .extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
        }
        // Check for fragment (FU-A)
        else if nal_header_type == 28u8 {
            debug!("Fragment started!! ----- ");
            self.is_fragment_start = true;

            // Fragment header (2nd NAL unit byte)
            //  +---------------+
            // |0|1|2|3|4|5|6|7| bit position
            // +-+-+-+-+-+-+-+-+
            // |S|E|R|  Type   |
            // +---------------+
            // S = Start of fragment?
            // E = End of fragment?

            // Check fragment header which is byte
            // after NAL header
            let header_frag = &self.buf_rtp[13];
            debug!("Fragment header -- {:08b}", header_frag);

            // Or fragment END?
            if *header_frag & 0b01000000 == 64u8 {
                trace!("Fragment ended!! ----- ");
                self.is_fragment_end = true;

                // Reconstruct new NAL header using NAL
                // NAL unit type in FRAGMENT header
                // AND NAL priority from original NAL header
                // use bitmasks to get first 3 bits and last 5 bits
                let nal_header = *header_frag & 0b00011111;
                let nal_header = nal_header | 0b01100000;
                debug!("New NAL header for conbined fragment: {:08b}", nal_header);

                self.buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
                // Need to swap outside nal header to inside payload type
                // as after combining packet it's not a fragment anymore
                // TODO: Need to get this from fragment header type instead of hard coding
                self.buf_temp.push(nal_header);
                self.buf_temp
                    .extend_from_slice(self.buf_fragments.as_slice());
                self.buf_temp.extend_from_slice(&self.buf_rtp[14..len]);
                self.buf_fragments.clear();
            } else {
                // Append fragment payload EXCLUDING ALL HEADERS
                self.buf_fragments.extend_from_slice(&self.buf_rtp[14..len]);
            }
        } else {
            debug!("Slice packet ----- ");

            self.is_sps_found = false;
            self.buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
            self.buf_temp
                .extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
        }

        Ok(())
    }

    pub fn try_decode(&mut self) -> Result<Option<DecodedYUV>, openh264::Error> {
        if self.buf_temp.len() == 0 || !self.is_start_decoding {
            return Ok(None);
        } else if self.is_fragment_start && !self.is_fragment_end {
            return Ok(None);
        }

        // Clear fragment flags
        self.is_fragment_start = false;
        self.is_fragment_end = false;

        // all current packets data
        self.buf_all.extend_from_slice(self.buf_temp.as_slice());

        // DECODE
        // Idea is to store all packets depending on types in buf_temp
        // SPS/PPS     = 2 packets
        // Fragment    = 1 packet COMBINED
        // Slice       = 1 packet
        debug!("//////////////////////////////////////////");
        debug!("Decoding packet size: {:?}", self.buf_temp.len());

        let maybe_some_yuv = match &mut self.decoder {
            Some(rtp_decoder) => rtp_decoder.decode(self.buf_temp.as_slice()),
            None => Err(openh264::Error::msg("Unable to decode NAL unit")),
        };

        self.buf_temp.clear();

        maybe_some_yuv
    }
}

fn get_nal_type(nal: u8) -> String {
    let nal_types = r#"0:Unspecified:non-VCL
        1:Coded slice of a non-IDR picture slice_layer_without_partitioning_rbsp():VCL
        2:Coded slice data partition A slice_data_partition_a_layer_rbsp():VCL
        3:Coded slice data partition B slice_data_partition_b_layer_rbsp():VCL
        4:Coded slice data partition C slice_data_partition_c_layer_rbsp():VCL
        5:Coded slice of an IDR picture slice_layer_without_partitioning_rbsp():VCL
        6:Supplemental enhancement information (SEI) sei_rbsp():non-VCL
        7:Sequence parameter set seq_parameter_set_rbsp():non-VCL
        8:Picture parameter set pic_parameter_set_rbsp():non-VCL
        9:Access unit delimiter access_unit_delimiter_rbsp():non-VCL
        10:End of sequence end_of_seq_rbsp():non-VCL
        11:End of stream end_of_stream_rbsp():non-VCL
        12:Filler data filler_data_rbsp():non-VCL
        13:Sequence parameter set extension seq_parameter_set_extension_rbsp():non-VCL
        14:Prefix NAL unit prefix_nal_unit_rbsp():non-VCL
        15:Subset sequence parameter set subset_seq_parameter_set_rbsp():non-VCL
        16:Reserved:non-VCL
        18:Reserved:non-VCL
        19:Coded slice of an auxiliary coded picture without partitioning slice_layer_without_partitioning_rbsp():non-VCL
        20:Coded slice extension slice_layer_extension_rbsp():non-VCL
        21:Coded slice extension for depth view components slice_layer_extension_rbsp() (specified in Annex I):non-VCL
        22:Reserved:non-VCL
        23:Reserved:non-VCL
        24:STAP-A:non-VCL
        25:STAP-B:non-VCL
        26:MTAP16:non-VCL
        27:MTAP24:non-VCL
        28:FU-A:non-VCL
        29:FU-B:non-VCL
        30:reserved:non-VCL
        31:reserved:non-VCL"#;

    nal_types
        .lines()
        .enumerate()
        .filter(|(i, _)| *i as u8 == nal)
        .map(|(_, line)| line.split(':').collect::<Vec<&str>>()[1])
        .collect::<String>()
}

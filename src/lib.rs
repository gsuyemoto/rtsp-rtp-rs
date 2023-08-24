use anyhow::{Error, Result};
use tokio::io::{ErrorKind, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use openh264::decoder::{Decoder, DecodedYUV};
use log::{debug, info, trace};
use std::fs::File;
use std::path::Path;
use url::Url;

pub enum RtpDecoders {
    OpenH264,
}

pub enum Methods {
    Options,
    Describe,
    Setup,
    Play,
    Teardown,
}

// Debated naming this Rtsp as I was thinking
// it could be confusing with Rtp being so
// close in spelling, but tried it with another
// name and felt it was worse
pub struct Rtsp {
    pub response_ok: bool,
    pub server_addr_rtp: Option<SocketAddr>,
    pub client_port_rtp: u16, // our port which server will send RTP
    server_addr_rtsp: SocketAddr,
    response_txt: String,
    cseq: u32,
    tcp_addr: SocketAddr,
    stream: TcpStream,
    transport: String,
    track: String,
    id: String,
}

pub struct Rtp {
    socket: UdpSocket,
    addr_client: SocketAddr,
    addr_server: SocketAddr,
    type_decoder: Option<RtpDecoders>,
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
    pub async fn new(client_ip: Option<&str>, client_port: u16, addr_server: SocketAddr) -> Result<Self> {
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

    pub async fn connect(&mut self, decoder: RtpDecoders) -> Result<()> {
        match decoder {
            RtpDecoders::OpenH264 => {
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

    pub fn save_file(&self) {
        let path = Path::new("video.h264");
        let display = path.display();
    
        // Open a file in write-only mode, returns `io::Result<File>`
        let mut file = match File::create(&path) {
            Err(why) => panic!("couldn't create {}: {}", display, why),
            Ok(file) => file,
        };
    
        match file.write_all(&self.buf_all) {
            Err(why) => panic!("couldn't write to {}: {}", display, why),
            Ok(_) => info!("successfully wrote to {}", display),
        }
    }

    pub async fn get_rtp(&mut self) -> Result<()> {
        let len = self.socket.recv(&mut self.buf_rtp).await?;
        // Byte 12 is NAL unit header (because of 0 index)
        // Previous bytes are RTP header
        // 13th byte is NAL header which in 0 index array = 12
        let header_nal = &self.buf_rtp[NAL_UNIT_START];

        info!("{} bytes received", len);
        info!("-----------\n{:08b}", header_nal);

        // Check if this is an SPS packet
        // NAL header byte -> 01100111
        if *header_nal == 103u8 {
            trace!("Sequence started! --------------------------------------");

            self.is_sps_found = true;
            self.buf_sps.extend_from_slice(&[0u8, 0u8, 0u8, 1u8]);
            self.buf_sps.extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
        }
        // Check if this is an PPS packet
        // NAL header byte -> 01101000
        else if *header_nal == 104u8 {
            info!("PPS packet ----- ");

            if self.is_sps_found {
                self.is_start_decoding = true;

                self.buf_temp.extend_from_slice(self.buf_sps.as_slice());
                self.buf_temp.extend_from_slice(&[0u8, 0u8, 0u8, 1u8]);
                self.buf_temp.extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
                self.buf_sps.clear();
            }
        }
        // Check if this is an SEI packet
        // NAL header byte -> 00000110
        else if *header_nal == 6u8 {
            info!("SEI packet ----- ");

            self.buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
            self.buf_temp.extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
        }
        // Check for fragment (FU-A)
        // NAL header byte -> 01111100
        else if *header_nal == 124u8 {
            info!("Fragment started!! ----- ");
            self.is_fragment_start = true;

            //  +---------------+
            // |0|1|2|3|4|5|6|7|
            // +-+-+-+-+-+-+-+-+
            // |S|E|R|  Type   |
            // +---------------+
            // S = Start
            // E = End

            // Check fragment header which is byte
            // after NAL header
            let header_frag = &self.buf_rtp[13];
            info!("Fragment header -- {:08b}", header_frag);

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
                self.buf_temp.extend_from_slice(self.buf_fragments.as_slice());
                self.buf_temp.extend_from_slice(&self.buf_rtp[14..len]);
                self.buf_fragments.clear();
            } else {
                // Append fragment payload EXCLUDING ALL HEADERS
                self.buf_fragments.extend_from_slice(&self.buf_rtp[14..len]);
            }
        } else {
            info!("Slice packet ----- ");

            self.is_sps_found = false;
            self.buf_temp.extend_from_slice(&[0u8, 0u8, 1u8]);
            self.buf_temp.extend_from_slice(&self.buf_rtp[NAL_UNIT_START..len]);
        }

        Ok(())
    }

    pub fn try_decode(&mut self) -> Result<Option<DecodedYUV>, openh264::Error> {
        if self.buf_temp.len() == 0 || !self.is_start_decoding {
            return Ok(None)
        }
        else if self.is_fragment_start && !self.is_fragment_end {
            return Ok(None)
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
        info!("//////////////////////////////////////////");
        info!("Decoding packet size: {:?}", self.buf_temp.len());

        let maybe_some_yuv = match &mut self.decoder {
            Some(rtp_decoder) => rtp_decoder.decode(self.buf_temp.as_slice()), 
            None => Err(openh264::Error::msg("Unable to decode NAL unit")),
        };
        
        self.buf_temp.clear();

        maybe_some_yuv
    }
}

impl Rtsp {
    pub async fn new(addr: &str, port_rtp: Option<u16>) -> Result<Self, Error> {
        let client_port_rtp = match port_rtp {
            Some(port) => port,
            None => 4588u16, // choose a sensible default
        };
        
        let socket_addr = match Url::parse(addr) {
            Ok(parsed_addr) => parsed_addr.socket_addrs(|| None)?,
            Err(e) => panic!("[Rtsp] Trying to parse {addr} resulted in {e}"),    
        };
        
        let tcp_stream = TcpStream::connect(socket_addr[0]).await?;

        println!("[Rtsp] Connecting to server at: {}", socket_addr[0]);

        Ok(Rtsp {
            response_ok: false,
            server_addr_rtp: None,
            server_addr_rtsp: socket_addr[0],
            client_port_rtp,
            response_txt: String::new(),
            tcp_addr: socket_addr[0],
            stream: tcp_stream,
            transport: String::new(),
            track: String::new(),
            id: String::new(),
            cseq: 1,
        })
    }

    #[rustfmt::skip]
    pub async fn send(&mut self, method_in: Methods) -> Result<&mut Self> {
        let method_str = match method_in {
            Methods::Options     => "OPTIONS",
            Methods::Describe    => "DESCRIBE",
            Methods::Setup       => "SETUP",
            Methods::Play        => "PLAY",
            Methods::Teardown    => "TEARDOWN",
        };

        // I think you need to append the token received in SETUP
        // response here? With my test camera, it wasn't needed

        // Add headers to request for different methods
        match method_in {
            Methods::Options     => {
                println!("[Rtsp][send] Message::Options sending...");    
            }
            Methods::Describe    => {
                println!("[Rtsp][send] Message::Describe sending...");    
            }
            Methods::Setup       => {
                println!("[Rtsp][send] Message::Setup sending...");    
                let video_codec = "RTP/AVP/UDP";
                let uni_multicast = "unicast";
                // Client port is port you are telling server that it needs to send RTP
                // traffic to. Add +1 to selected port for RTCP traffic. This is by
                // convention and recommended in RFC.
                let client_port = format!("{}-{}", self.client_port_rtp, self.client_port_rtp +1);
                
                self.transport = format!("Transport: {};{};client_port={}\r\n",
                    video_codec,
                    uni_multicast,
                    client_port);
                self.track = "/trackID=0\r\n".to_string();
            }
            Methods::Play        => {
                println!("[Rtsp][send] Message::Play sending...");    
                self.transport = String::new();
                self.track = String::new();
            }
            Methods::Teardown    => {
                println!("[Rtsp][send] Message::Teardown sending...");    
            }
        }

        let request = format!(
            "{} {}{} RTSP/1.0\r\nCSeq: {}\r\n{}{}\r\n",
            method_str, 
            self.tcp_addr, 
            self.track, 
            self.cseq, 
            self.transport, 
            self.id,
        );

        let mut buf = Vec::with_capacity(4096);
        let mut buf_size: usize = 0;

        // Send command with proper headers
        // every command must provide cseq
        // which is incremented sequence as a header
        self.stream.write_all(request.as_bytes()).await?;

        'read: loop {
            // Wait for the socket to be readable
            self.stream.readable().await?;

            // Try to read data, this may still fail with `WouldBlock`
            // if the readiness event is a false positive.
            match self.stream.try_read_buf(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf_size = n;
                    break 'read;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }

        self.cseq += 1;
        self.check_ok(&buf[..buf_size], method_str);
        
        match method_in {
            Methods::Options     => (),
            Methods::Describe    => self.parse_describe(),
            Methods::Setup       => self.parse_setup(),
            Methods::Play        => (),
            Methods::Teardown    => self.parse_stop(),
        }

        Ok(self)
    }

    fn check_ok(&mut self, response: &[u8], method: &str) {
        let response = (*String::from_utf8_lossy(&response)).to_string();

        if *&response.len() == 0 {
            eprintln!("[Rtsp][send] {method} Response is empty.");
        }
        else {
            debug!("//--------------------- {method} RESPONSE");
            debug!("{:#?}", &response);
        }

        self.response_ok = (&response).contains("200 OK");
        self.response_txt = response;
    }

    // Parse OPTIONS methods to determine available methods/commands
    // fn parse_options(&mut self) {}
    // fn parse_play(&mut self) {}

    fn parse_describe(&mut self) {
        // SDP data begins after \r\n\r\n
        let (_headers, sdp) = self.response_txt.split_once("\r\n\r\n").unwrap();
        let sdp_fields = sdp.lines();

        debug!("SDP ///---------------\n{:?}", sdp_fields);
    }

    fn parse_setup(&mut self) {
        let resp_headers = self.response_txt.lines();

        // Parse response from SETUP command
        let setup_hash: HashMap<&str, &str> = resp_headers
            .into_iter()
            .filter(|line| line.contains(":"))
            .map(|line| line.split(": ").collect::<Vec<&str>>())
            .map(|v| (v[0], v[1]))
            .collect();

        // Parse the Transport header of the response
        // which contains:
        // 'server_port'
        // 'ssrc'
        // 'source' => server IP
        let transport_hash: HashMap<&str, &str> = setup_hash
            .get("Transport")
            .unwrap()
            .split(';')
            .collect::<Vec<&str>>()
            .iter()
            .filter(|s| s.contains('='))
            .map(|line| line.split('=').collect::<Vec<&str>>())
            .map(|v| (v[0], v[1]))
            .collect();

        // Create a new server socket address to talk to it via RTP
        // The address will have the same IP, but the port is sent
        // via the 'SETUP' command
        let server_port = transport_hash.get("server_port")
            .expect("[RTSP][parse_setup] Error finding server_port in response");

        // server_port returns port range (e.g. 6600-6601)
        // first port is RTP port
        // second port is RTCP port
        let server_rtp_rtcp: Vec<&str> = server_port.split('-').collect(); 

        // We've been talking to server as something like 192.168.1.100:554
        // Just remove the '554' port and replace with response in SETUP
        let mut server_addr = self.server_addr_rtsp.clone();
        server_addr.set_port(server_rtp_rtcp[0].parse::<u16>()
            .expect("[RTSP][parse_setup] Error parsing server_port"));

        self.server_addr_rtp = Some(server_addr);
        self.id = format!("Session: {}", setup_hash.get("Session")
            .expect("[RTSP][parse_setup] Error getting Session from hash"));
    }

    fn parse_stop(&mut self) {
        match self.response_ok {
            true  => println!("Shutdown Ok"),
            false => eprintln!("Shutdown Error"),
        }
    }
}

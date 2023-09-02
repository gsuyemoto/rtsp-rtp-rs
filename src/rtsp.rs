use anyhow::Result;
use url::Url;
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, ErrorKind};
use log::debug;
use std::collections::HashMap;
use std::net::SocketAddr;

pub enum Methods {
    Options,
    Describe,
    Setup,
    Play,
    Teardown,
}

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

impl Rtsp {
    pub async fn new(addr: &str, port_rtp: Option<u16>) -> Result<Self> {
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
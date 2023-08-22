use anyhow::{Error, Result};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};

pub enum Methods {
    Options,
    Describe,
    Setup,
    Play,
    Teardown,
}

#[derive(Debug)]
pub struct Session {
    cseq: u32,
    server_addr: String,
    stream: TcpStream,
    transport: String,
    track: String,
    buf_size: usize,
    id: String,
}

pub struct Response {
    pub msg: String,
    pub ok: bool,
    raw_response: [u8; 1024],
    session_id: Option<String>,
}

impl Response {
    pub fn new(raw_response: [u8; 1024]) -> Self {
        Response {
            raw_response,
            msg: String::new(),
            ok: false,
            session_id: None,
        }    
    }

    fn init(self, msg_type: Methods) -> Self {
        let str_response = (*String::from_utf8_lossy(&self.raw_response)).to_string();

        // Some responses come with specially formatted
        // data that depends on type of command sent
        match msg_type {
            Methods::Options     => self,
            Methods::Describe    => self.parse_describe(str_response),
            Methods::Setup       => self.parse_setup(str_response),
            Methods::Play        => self.parse_play(str_response),
            Methods::Teardown    => self,
        }
    }

    fn parse_play(mut self, str_response: String) -> Self {
        self.ok = (&str_response).contains("200 OK");
        self.msg = str_response;

        self
    }

    fn parse_describe(mut self, str_response: String) -> Self {
        // SDP data begins after \r\n\r\n
        let (headers, sdp) = str_response.split_once("\r\n\r\n").unwrap();
        let sdp_fields = sdp.lines();
        
        self.ok = (&str_response).contains("200 OK");
        self.msg = str_response;

        self
    }

    fn parse_setup(mut self, str_response: String) -> Self {
        let resp_headers = str_response.lines();

        let session_id = resp_headers
            .into_iter()
            .filter(|line| line.contains("Session"))
            .map(|line| line.split(|c| c == ':' || c == ';').collect::<Vec<&str>>())
            .map(|v| v[1])
            .collect::<String>();

        self.ok = (&str_response).contains("200 OK");
        self.msg = str_response;
        self.session_id = Some(format!("Session: {session_id}"));

        self
    }
}

impl Session {
    pub fn new(server_addr: String) -> Result<Self, Error> {
        let tcp_stream = TcpStream::connect(&server_addr)?;

        // Indicate in the Transport heading whether you want TCP/UDP
        // With this camera it seems when TCP is chosen, then the
        // server will NOT respond with a port number. I guess this
        // means that it uses existing TCP connection to send RTP?
        // When UDP is chosen, a port is provided in response. With
        // this camera (Topodome) choosing UDP provided a port in
        // the response at 6600.

        // I think you need to append the token received in SETUP
        // response here. With my test camera, it was not necessary
        // and without the token, I still received 200 OK

        Ok(Session {
            server_addr,
            stream: tcp_stream,
            transport: String::new(),
            track: String::new(),
            id: String::new(),
            cseq: 1,
            buf_size: 1024,
        })
    }

    #[rustfmt::skip]
    pub async fn send(&mut self, method_in: Methods) -> Result<Response> {
        let method_str = match method_in {
            Methods::Options     => "OPTIONS",
            Methods::Describe    => "DESCRIBE",
            Methods::Setup       => "SETUP",
            Methods::Play        => "PLAY",
            Methods::Teardown    => "TEARDOWN",
        };

        // Need to add headers to request for different methods
        match method_in {
            Methods::Options     => (),
            Methods::Describe    => (),
            Methods::Setup       => {
                                        self.transport =
                                            "Transport: RTP/AVP/UDP;unicast;client_port=4588-4589\r\n".to_string();
                                        self.track = "/trackID=0\r\n".to_string();
                                    }
            Methods::Play        => {
                                        self.transport = String::new();
                                        self.track = String::new();
                                    }
            Methods::Teardown    => (),
        }

        let request = format!(
            "{} {}{} RTSP/1.0\r\nCSeq: {}\r\n{}{}\r\n",
            method_str, 
            self.server_addr, 
            self.track, 
            self.cseq, 
            self.transport, 
            self.id,
        );

        // let mut buffer = Vec::with_capacity(self.buf_size);
        let mut buffer = [0u8; 1024];

        // Send command with proper headers
        // every command must provide cseq
        // which is incremented sequence as a header
        self.stream.write(request.as_bytes())?;
        let resp_size = self.stream.read(&mut buffer)?;
        self.cseq += 1;

        Ok(Response::new(buffer).init(method_in))
    }

    pub fn stop(&mut self) -> Result<Response> {
        let request = format!(
            "TEARDOWN {} RTSP/1.0\r\nCSeq: {}\r\n{}\r\n",
            self.server_addr, self.cseq, self.id,
        );

        // let mut buffer = Vec::with_capacity(self.buf_size);
        let mut buffer = [0u8; 1024];

        let response = self.stream.write(request.as_bytes())?;
        let resp_size = self.stream.read(&mut buffer)?;
        let response = Response::new(buffer);

        if response.ok {
            match self.stream.shutdown(Shutdown::Both) {
                Ok(_) => println!("Shutdown Ok"),
                Err(e) => eprintln!("Shutdown Error: {e}"),
            }
        }

        Ok(response)
    }
}

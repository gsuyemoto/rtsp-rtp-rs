use anyhow::Result;
use log::{info, trace, warn};
use rtsp_rtp_rs::rtp::{Decoders, Rtp};
use rtsp_rtp_rs::rtsp::{Methods, Rtsp};
//------------------SDL2
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    // let mut rtsp = Rtsp::new("rtsp://192.168.86.218:554/11", None).await?;
    let mut rtsp = Rtsp::new("rtsp://192.168.86.141:554/11", None).await?;

    rtsp.send(Methods::Options)
        .await?
        .send(Methods::Describe)
        .await?
        .send(Methods::Setup)
        .await?
        .send(Methods::Play)
        .await?;

    if rtsp.response_ok {
        // Bind address will default to "0.0.0.0"
        // Bind port was defined in RTSP 'SETUP' command

        let mut rtp_stream =
            Rtp::new(None, rtsp.client_port_rtp, rtsp.server_addr_rtp.unwrap()).await?;
        rtp_stream.connect(Decoders::OpenH264).await?;

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

        // Need this during testing as the first 40 frames
        // or so are blank because it's not starting from SPS
        // and instead getting frames from mid stream

        'read_rtp_packets: loop {
            for event in event_pump.poll_iter() {
                match event {
                    Event::Quit { .. }
                    | Event::KeyDown {
                        keycode: Some(Keycode::Escape),
                        ..
                    } => break 'read_rtp_packets,
                    _ => {}
                }
            }

            rtp_stream.get_rtp().await?;

            let maybe_some_yuv = rtp_stream.try_decode();
            match maybe_some_yuv {
                Ok(some_yuv) => match some_yuv {
                    Some(yuv) => {
                        trace!("Decoded YUV!");

                        let (y_size, u_size, v_size) = yuv.strides_yuv();
                        let _result = texture.update_yuv(
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
                    None => trace!("Unable to decode to YUV"),
                },
                // Have been unable to decipher OpenH264 error codes
                // Instead, used ffprobe to get errors pertaining to malformed streams
                // save to raw h264 file and then ffprobe or ffplay (FFMPEG)
                Err(e) => warn!("Error: {e}"),
            }
        }
    }

    #[rustfmt::skip]
    let is_ok = rtsp
        .send(Methods::Teardown)
        .await?
        .response_ok;

    info!("Stopping RTSP: {}", is_ok);
    Ok(())
}

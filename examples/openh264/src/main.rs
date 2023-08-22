use anyhow::Result;
use log::{debug, info, trace, warn};
use rtsp_client::{Methods, Rtp, RtpDecoders, Session};
use std::io::prelude::*;
//------------------SDL2
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let mut rtsp = Session::new("192.168.86.112:554")?;

    rtsp.send(Methods::Options)
        .await?
        .send(Methods::Describe)
        .await?
        .send(Methods::Setup)
        .await?
        .send(Methods::Play)
        .await?;

    if rtsp.response_ok() {
        let mut rtp_stream = Rtp::new(4588).await?;
        rtp_stream.connect(RtpDecoders::OpenH264).await?;

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
    }

    info!("Stopping RTSP: {}", rtsp.stop()?.ok);
    Ok(())
}

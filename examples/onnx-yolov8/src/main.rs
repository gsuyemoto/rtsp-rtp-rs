use anyhow::Result;
use log::{info, warn};
use onvif_cam_rs::client::{Client, Messages};
use rtsp_rtp_rs::rtp::{Decoders, Rtp};
use rtsp_rtp_rs::rtsp::{Methods, Rtsp};
//------------------SDL2
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
//------------------YoloV8
use ndarray::{s, Array, Axis, IxDyn};
use ort::{Environment, SessionBuilder, Value};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    // Find an IP cam and get it's RTP URI using
    // device discovery and ONVIF
    let mut onvif_client = Client::new().await;
    let _ = onvif_client.send(Messages::Capabilities, 1).await?;
    let _ = onvif_client.send(Messages::DeviceInfo, 1).await?;
    let _ = onvif_client.send(Messages::Profiles, 1).await?;
    let rtsp_addr = onvif_client.send(Messages::GetStreamURI, 1).await?;

    // If using IP cams, this can be discovered via Onvif
    // if the camera supports it
    let mut rtsp = Rtsp::new(&rtsp_addr, None).await?;

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
        let mut wait_frames = 0;

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

                        wait_frames += 1;

                        if wait_frames > 40 && wait_frames % 100 == 0 {
                            let mut buf_rgb = [0u8; 640 * 352 * 4]; // rgba
                            yuv.write_rgba8(&mut buf_rgb[..]);
                            let buf_rgb = buf_rgb.to_vec();

                            let boxes = detect_objects_on_image(buf_rgb);
                            println!(
                                "Detection: {}",
                                if boxes.len() > 0 { boxes[0].4 } else { "None" }
                            );
                        }

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
                    None => info!("Unable to decode to YUV"),
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

// Function receives an image,
// passes it through YOLOv8 neural network
// and returns an array of detected objects
// and their bounding boxes
// Returns Array of bounding boxes in format [(x1,y1,x2,y2,object_type,probability),..]
fn detect_objects_on_image(buf: Vec<u8>) -> Vec<(f32, f32, f32, f32, &'static str, f32)> {
    let (input, img_width, img_height) = prepare_input(buf);
    let output = run_model(input);
    return process_output(output, img_width, img_height);
}

// Function used to convert input image to tensor,
// required as an input to YOLOv8 object detection
// network.
// Returns the input tensor, original image width and height
fn prepare_input(buf: Vec<u8>) -> (Array<f32, IxDyn>, u32, u32) {
    let mut input = Array::zeros((1, 3, 640, 640)).into_dyn();

    let mut x: usize = 0;
    let mut y: usize = 0;
    buf.chunks(4).for_each(|chunk| {
        input[[0, 0, y, x]] = chunk[0] as f32 / 255.0; // R
        input[[0, 1, y, x]] = chunk[1] as f32 / 255.0; // G
        input[[0, 2, y, x]] = chunk[2] as f32 / 255.0; // B

        // X and Y coords from flat Vec
        x += 1;
        if x == 640 {
            x = 0;
            y += 1;
        }
    });

    // Hardcoded to each frame image size
    // TODO: Change this to receive size dynamically
    return (input, 640, 352);
}

// Function used to pass provided input tensor to
// YOLOv8 neural network and return result
// Returns raw output of YOLOv8 network as a single dimension
// array
fn run_model(input: Array<f32, IxDyn>) -> Array<f32, IxDyn> {
    let env = Arc::new(Environment::builder().with_name("YOLOv8").build().unwrap());
    let model = SessionBuilder::new(&env)
        .unwrap()
        .with_model_from_file("yolov8m.onnx")
        .unwrap();
    let input_as_values = &input.as_standard_layout();
    let model_inputs = vec![Value::from_array(model.allocator(), input_as_values).unwrap()];
    let outputs = model.run(model_inputs).unwrap();
    let output = outputs
        .get(0)
        .unwrap()
        .try_extract::<f32>()
        .unwrap()
        .view()
        .t()
        .into_owned();
    return output;
}

// Function used to convert RAW output from YOLOv8 to an array
// of detected objects. Each object contain the bounding box of
// this object, the type of object and the probability
// Returns array of detected objects in a format [(x1,y1,x2,y2,object_type,probability),..]
fn process_output(
    output: Array<f32, IxDyn>,
    img_width: u32,
    img_height: u32,
) -> Vec<(f32, f32, f32, f32, &'static str, f32)> {
    let mut boxes = Vec::new();
    let output = output.slice(s![.., .., 0]);
    for row in output.axis_iter(Axis(0)) {
        let row: Vec<_> = row.iter().map(|x| *x).collect();
        let (class_id, prob) = row
            .iter()
            .skip(4)
            .enumerate()
            .map(|(index, value)| (index, *value))
            .reduce(|accum, row| if row.1 > accum.1 { row } else { accum })
            .unwrap();
        if prob < 0.5 {
            continue;
        }
        let label = YOLO_CLASSES[class_id];
        let xc = row[0] / 640.0 * (img_width as f32);
        let yc = row[1] / 640.0 * (img_height as f32);
        let w = row[2] / 640.0 * (img_width as f32);
        let h = row[3] / 640.0 * (img_height as f32);
        let x1 = xc - w / 2.0;
        let x2 = xc + w / 2.0;
        let y1 = yc - h / 2.0;
        let y2 = yc + h / 2.0;
        boxes.push((x1, y1, x2, y2, label, prob));
    }

    boxes.sort_by(|box1, box2| box2.5.total_cmp(&box1.5));
    let mut result = Vec::new();
    while boxes.len() > 0 {
        result.push(boxes[0]);
        boxes = boxes
            .iter()
            .filter(|box1| iou(&boxes[0], box1) < 0.7)
            .map(|x| *x)
            .collect()
    }
    return result;
}

// Function calculates "Intersection-over-union" coefficient for specified two boxes
// https://pyimagesearch.com/2016/11/07/intersection-over-union-iou-for-object-detection/.
// Returns Intersection over union ratio as a float number
fn iou(
    box1: &(f32, f32, f32, f32, &'static str, f32),
    box2: &(f32, f32, f32, f32, &'static str, f32),
) -> f32 {
    return intersection(box1, box2) / union(box1, box2);
}

// Function calculates union area of two boxes
// Returns Area of the boxes union as a float number
fn union(
    box1: &(f32, f32, f32, f32, &'static str, f32),
    box2: &(f32, f32, f32, f32, &'static str, f32),
) -> f32 {
    let (box1_x1, box1_y1, box1_x2, box1_y2, _, _) = *box1;
    let (box2_x1, box2_y1, box2_x2, box2_y2, _, _) = *box2;
    let box1_area = (box1_x2 - box1_x1) * (box1_y2 - box1_y1);
    let box2_area = (box2_x2 - box2_x1) * (box2_y2 - box2_y1);
    return box1_area + box2_area - intersection(box1, box2);
}

// Function calculates intersection area of two boxes
// Returns Area of intersection of the boxes as a float number
fn intersection(
    box1: &(f32, f32, f32, f32, &'static str, f32),
    box2: &(f32, f32, f32, f32, &'static str, f32),
) -> f32 {
    let (box1_x1, box1_y1, box1_x2, box1_y2, _, _) = *box1;
    let (box2_x1, box2_y1, box2_x2, box2_y2, _, _) = *box2;
    let x1 = box1_x1.max(box2_x1);
    let y1 = box1_y1.max(box2_y1);
    let x2 = box1_x2.min(box2_x2);
    let y2 = box1_y2.min(box2_y2);
    return (x2 - x1) * (y2 - y1);
}

// Array of YOLOv8 class labels
const YOLO_CLASSES: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

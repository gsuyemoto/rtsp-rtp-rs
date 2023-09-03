Implementation of RTSP and RTP protocols. Each protocol implementation is very barebones and with a very narrow focus right now. 

The following commands are barely implemented:

* Options
* Describe
* Setup
* Play
* Teardown

This lib is best used with it's sister implementation for ONVIF discovery: https://github.com/gsuyemoto/onvif-cam-rs.

Very early development and with probably breaking API changes often. This lib has only been test to work with a single IP camera from Amazon -- a Topodome fixed IP camera which supports ONVIF.

Currently, the lib supports only software decoding of H264 using the [OpenH264 crate for Rust]: https://crates.io/crates/openh264. I have been working on implementing Libva for hardware accelerated decoding of H264. Not even sure which H264 profiles it will work with other than baseline as that's the only profile it's been tested on.

Please see the example for usage. The example connects to an IP camera using the my [ONVIF lib]: https://github.com/gsuyemoto/onvif-cam-rs and [Andrey Germanov's YoloV8 code using ONNX]: https://github.com/AndreyGermanov/yolov8_onnx_rust. Obviously, my example is a very naive use of his code and so any poor performance is assuredly due to the haphazard way in which I tried it out with the IP camera. Just wanted to see if I could get it working. Even at it's bad frame rate, it's pretty cool to have YoloV8, a state of the art object recognition algo running on my home IP camera...

The example has only been test on my Ubuntu 22 machine. Running the example will require SDL2 to be available:

```bash
sudo apt-get install libsdl2-dev
```

Again, the cool YoloV8 example is due to Andrey. I tweaked his code to parse the RGBA image after OpenH264 converted it from YUV to RGBA and updated it to use the lates ORT crate. The example is GPL 3.0 due to Andrey's licensing.

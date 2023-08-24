This repo was made primarily to aid in my use of IP cameras. This example works best with another quick repo I developed to provide device discovery via ONVIF. You can skip the device discovery and provide the IP of the camera yourself. If you want to use the device discovery, you will need to clone the onvif repo at https://github.com/gsuyemoto/onvif-client-rs and then change the path in the Cargo.toml to point to the cloned directory's path if necessary.

Very early development. I've only tested this on one camera, a Topodome IP camera I purchased on Amazon. Camera needs to support ONVIF for discovery. For streaming, I only have implemented the H264 codec, using the OpenH264 from Cisco. Hope to implement more codecs soon.

The example implements Yolov8 using an ONNX model and the ORT crate. It's frame rate is pretty horrid right now because of Yolov8. Hope to improve this with some GPU targeting and some other optimizations. I also have a naive implementation of the Yolov8 and that can definitely be improved.

The RTSP and RTP protocols are implemented mostly from scratch. Each protocol is far from complete. Also, I had to do a bit of NAL unit processing in order to convert the RTP packets to a streaming format that OpenH264 can understand.

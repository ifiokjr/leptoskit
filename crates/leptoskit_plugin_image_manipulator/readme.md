# leptoskit_plugin_image_manipulator

Decode, resize, crop, rotate, flip and encode JPEG/PNG images using one Rust implementation on iOS, Android, desktop and browser WASM. The native plugin registers on the Rust worker. A batch of operations takes one bridge call.

```rust
use leptoskit_plugin_image_manipulator::{Bitmap, Filter, Operation, Output, encode};

# fn example() -> Result<(), leptoskit_bridge::Error> {
let pixels = Bitmap::from_rgba(1, 1, vec![40, 120, 200, 255])?;
let resized = pixels.transform(&[Operation::Resize {
    width: Some(64),
    height: None,
    filter: Filter::Bilinear,
}])?;
let png = encode(&resized, Output::Png)?;
assert_eq!(png.info.width, 64);
# Ok(())
# }
```

`inspect` reads dimensions without decoding pixels. `decode` returns a bounded RGBA8 `Bitmap`; `Bitmap::transform` consumes it and applies operations in order. `encode` returns bytes and dimensions. `manipulate` combines these steps and validates the entire operation plan before decompressing pixels.

Native hosts and browser applications use the default `engine` feature. Interpreter guests disable it and use `guest::ImageManipulator::{new, inspect, manipulate}`:

```toml
[dependencies.leptoskit_plugin_image_manipulator]
version = "0.1"
default-features = false
features = ["guest"]
```

This guest configuration includes the binary protocol and allocation-free header inspection without building the image codec dependency graph. The direct Rust engine also compiles for browser WASM and requires no DOM or JavaScript image decoder. It is synchronous CPU work: browser applications should schedule substantial jobs in a Web Worker. Merely calling it from an async function does not move work off the browser UI thread.

Resize accepts exact width/height or derives one omitted dimension while preserving aspect ratio. Nearest-neighbour and bilinear filters are available; bilinear interpolation uses premultiplied alpha to prevent transparent colors bleeding into visible edges. Crop bounds are checked against the current image. Rotation supports 90°, 180° and 270° clockwise. Flips and 180° rotation operate in place. Identity resize/crop reuses the existing allocation. Other transforms allocate only the destination RGBA buffer, without float-image intermediates.

PNG output preserves alpha. JPEG output requires a quality of 1–100 and an explicit RGB background for alpha compositing. PNG/JPEG decode uses maintained [`image`](https://docs.rs/image/latest/image/) codecs with other formats and Rayon disabled. EXIF orientation is not applied automatically; operations address the stored pixel orientation. ICC profiles are not used for color conversion. All source metadata is omitted from encoded output. PNG animation is rejected.

| Bound                                       | Value                   |
| ------------------------------------------- | ----------------------- |
| Compressed input                            | 4 MiB                   |
| Encoded output                              | 4 MiB                   |
| Width or height                             | 8,192 pixels            |
| Source and every intermediate image         | 4,194,304 pixels        |
| Operations per request                      | 16                      |
| Transform-plan pixel work                   | 67,108,864 pixel visits |
| Owned-buffer preflight plus codec allowance | 64 MiB                  |
| PNG decoder allocation limit                | 8 MiB                   |

Image dimensions and pixel counts are checked **before constructing a pixel decoder**, including JPEG frame headers and PNG IHDR. The decoder receives strict dimension limits as a second check. Scratch preflight includes the worst supported 16-bit PNG buffer plus RGBA8 conversion, compressed-input retention, transform source/destination buffers, encoding scratch allowance and bounded output. The output writer fails before appending bytes beyond its budget. Oversized output fails rather than returning a truncated file.

The scratch budget is not process memory isolation: allocator overhead and codec-private allocations are not under this crate's allocator control. [`image::Limits::max_alloc`](https://docs.rs/image/latest/image/struct.Limits.html) is not universally strict; notably, its JPEG decoder ignores the allocation field. JPEG dimensions and input size are still strictly bounded. PNG compressed text stays compressed and is never requested by this API; it is omitted from output. Native code processing hostile images under a hard resident-memory requirement should use an isolated process with an OS memory limit.

The API covers core transformations from [Expo ImageManipulator](https://docs.expo.dev/versions/latest/sdk/imagemanipulator/). Arbitrary-angle rotation, WebP, animated images, URI loading, filesystem writes, persistent image handles and browser worker creation are not implemented. Callers supply bytes through their existing permitted file or network APIs.

## Binary protocol

`INSPECT` (0) accepts raw encoded bytes and returns three u32 little-endian values: format (0=PNG, 1=JPEG), width and height.

`MANIPULATE` (1) accepts output format; JPEG additionally includes quality and three background components as u32 values. It then contains an operation count, operations, and a u32-length-prefixed image. Operation tags are 0=resize (width, height, filter), 1=crop (x, y, width, height), 2=rotate (0=90°, 1=180°, 2=270°), 3=flip (0=horizontal, 1=vertical). A zero resize dimension means omitted; filter tags are 0=nearest and 1=bilinear.

The reply contains length-prefixed INSPECT information followed by length-prefixed encoded bytes. The guest validates the returned dimensions/format against the encoded header. All decoders reject unknown tags, truncation and trailing bytes.

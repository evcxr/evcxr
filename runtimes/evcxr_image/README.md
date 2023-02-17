# Evcxr image

Integration between [Evcxr
Jupyter](https://github.com/evcxr/evcxr/blob/main/evcxr_jupyter/README.md)
and the image crate. Enables display of images in Evcxr Jupyter kernel.

Currently supports all 8 bit per channel formats:

Example usage:
```rust
:dep image = "0.23"
:dep evcxr_image = "1.1"

use evcxr_image::ImageDisplay;

image::ImageBuffer::from_fn(256, 256, |x, y| {
    if (x as i32 - y as i32).abs() < 3 {
        image::Rgb([0, 0, 255])
    } else {
        image::Rgb([0, 0, 0])
    }
})
```

// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use image::ImageFormat;
use std::io::Cursor;
use std::ops::Deref;

pub trait ImageDisplay {
    fn evcxr_display(&self);
}

impl<P, C> ImageDisplay for image::ImageBuffer<P, C>
where
    P: image::PixelWithColorType,
    [P::Subpixel]: image::EncodableLayout,
    C: Deref<Target = [P::Subpixel]>,
{
    fn evcxr_display(&self) {
        let mut buffer = Cursor::new(Vec::new());
        self.write_to(&mut buffer, ImageFormat::Png).unwrap();
        evcxr_runtime::mime_type("image/png").bytes(buffer.get_ref());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rgb_image() {
        let img: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(10, 10, |x, y| {
                if (x as i32 - y as i32).abs() < 3 {
                    image::Rgb([0, 0, 255])
                } else {
                    image::Rgb([0, 0, 0])
                }
            });
        use super::ImageDisplay;
        img.evcxr_display();
    }

    #[test]
    fn gray_image() {
        let img: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(10, 10, |x, y| {
                if (x as i32 - y as i32).abs() < 3 {
                    image::Luma([255])
                } else {
                    image::Luma([0])
                }
            });
        use super::ImageDisplay;
        img.evcxr_display();
    }
}

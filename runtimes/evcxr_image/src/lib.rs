// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use std::ops::Deref;

pub trait ImageDisplay {
    fn evcxr_display(&self);
}

impl<P: image::Pixel<Subpixel = u8> + 'static, C> ImageDisplay for image::ImageBuffer<P, C>
where
    C: Deref<Target = [P::Subpixel]>,
{
    fn evcxr_display(&self) {
        let mut buffer = Vec::new();
        image::png::PngEncoder::new(&mut buffer)
            .encode(self, self.width(), self.height(), P::COLOR_TYPE)
            .unwrap();
        evcxr_runtime::mime_type("image/png").bytes(&buffer);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rgb_image() {
        let img = image::ImageBuffer::from_fn(10, 10, |x, y| {
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
        let img = image::ImageBuffer::from_fn(10, 10, |x, y| {
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

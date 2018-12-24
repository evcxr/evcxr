// Copyright 2018 Google Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

extern crate evcxr_runtime;
extern crate image;

pub trait ImageDisplay {
    fn evcxr_display(&self);
}

impl ImageDisplay for image::RgbImage {
    fn evcxr_display(&self) {
        let mut buffer = Vec::new();
        image::png::PNGEncoder::new(&mut buffer)
            .encode(
                &**self,
                self.width(),
                self.height(),
                image::ColorType::RGB(8),
            )
            .unwrap();
        evcxr_runtime::mime_type("image/png").bytes(&buffer);
    }
}

impl ImageDisplay for image::GrayImage {
    fn evcxr_display(&self) {
        let mut buffer = Vec::new();
        image::png::PNGEncoder::new(&mut buffer)
            .encode(
                &**self,
                self.width(),
                self.height(),
                image::ColorType::Gray(8),
            )
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

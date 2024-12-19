use std::time::Instant;

use openh264::{
    decoder::{Decoder, DecoderConfig},
    formats::YUVSource,
    OpenH264API,
};

#[derive(Debug)]
pub enum DecoderError {
    InitFail(openh264::Error),
    DecodeFail(openh264::Error),
    NoImageDecoded,
    FieldOutOfBounds,
    NalOutOfBounds,
    IndexOutOfBounds,
}

// TODO: Cant decide between caching the buffer into each decoder, or just create the vec in
// between decoders
pub trait ImageDecoder: Sync + Send {
    fn decode(&mut self, data: &[u8]) -> Result<&[u8], DecoderError>;
}

pub trait Chain<T: 'static + ImageDecoder> {
    fn chain(self, other: T) -> ChainedDecoder;
}

pub struct AVCCDecoder {
    buf: Vec<u8>,
}

impl AVCCDecoder {
    pub fn new() -> Self {
        return Self { buf: Vec::new() };
    }
}

impl ImageDecoder for AVCCDecoder {
    fn decode(&mut self, data: &[u8]) -> Result<&[u8], DecoderError> {
        let b = Instant::now();
        self.buf.clear();
        let mut index = 0;

        while index < data.len() {
            // Read the 4-byte size field
            if index + 4 > data.len() {
                return Err(DecoderError::FieldOutOfBounds);
            }

            let nal_size = u32::from_be_bytes([
                data[index],
                data[index + 1],
                data[index + 2],
                data[index + 3],
            ]) as usize;

            index += 4; // Skip the size field

            if index + nal_size > data.len() {
                return Err(DecoderError::NalOutOfBounds);
            }

            // Extract the NAL unit
            let nal_unit = &data[index..index + nal_size];
            index += nal_size;

            // Prepend the Annex B start code (0x00000001)
            self.buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            self.buf.extend_from_slice(nal_unit);
        }

        println!(
            "Avcc decoding time -> {}",
            Instant::now().duration_since(b).as_millis()
        );
        Ok(&self.buf)
    }
}

impl<T: 'static + ImageDecoder> Chain<T> for AVCCDecoder {
    fn chain(self, other: T) -> ChainedDecoder {
        ChainedDecoder {
            a: Box::new(self),
            b: Box::new(other),
        }
    }
}

pub struct H264RGBDecoder {
    inner: Decoder,
    buf: Vec<u8>,
}

impl H264RGBDecoder {
    pub fn new(dbg: bool, image_size: (usize, usize)) -> Result<Self, DecoderError> {
        let decoder =
            Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().debug(dbg))
                .map_err(|e| DecoderError::InitFail(e))?;
        Ok(Self {
            inner: decoder,
            buf: vec![0u8; image_size.0 * image_size.1 * 3],
        })
    }
}

impl ImageDecoder for H264RGBDecoder {
    fn decode(&mut self, data: &[u8]) -> Result<&[u8], DecoderError> {
        let bb = Instant::now();
        let a = self
            .inner
            .decode(&data)
            .map_err(|e| DecoderError::DecodeFail(e))
            .map(|o| o.ok_or(DecoderError::NoImageDecoded))?
            .map(|i| {
                let b = Instant::now();
                i.write_rgb8(&mut self.buf);
                println!(
                    "Took {} ms to write into rgb",
                    Instant::now().duration_since(b).as_millis()
                );
                self.buf.as_slice()
            });
        println!(
            "Took {} ms to decode image",
            Instant::now().duration_since(bb).as_millis()
        );
        a
    }
}
pub struct H264BGRDecoder {
    inner: Decoder,
    buf: Vec<u8>,
}

impl H264BGRDecoder {
    pub fn new(dbg: bool, image_size: (usize, usize)) -> Result<Self, DecoderError> {
        let decoder =
            Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().debug(dbg))
                .map_err(|e| DecoderError::InitFail(e))?;
        Ok(Self {
            inner: decoder,
            buf: vec![0u8; image_size.0 * image_size.1 * 3],
        })
    }
}

impl ImageDecoder for H264BGRDecoder {
    fn decode(&mut self, data: &[u8]) -> Result<&[u8], DecoderError> {
        let bb = Instant::now();
        let a = self
            .inner
            .decode(&data)
            .map_err(|e| DecoderError::DecodeFail(e))
            .map(|o| o.ok_or(DecoderError::NoImageDecoded))?
            .map(|i| {
                let b = Instant::now();
                let dim = i.dimensions_uv();
                let strides = i.strides();
                let wanted = dim.0 * dim.1 * 3;

                for y in 0..dim.1 {
                    for x in 0..dim.0 {
                        let base_tgt = (y * dim.0 + x) * 3;
                        let base_y = y * strides.0 + x;
                        let base_u = (y / 2 * strides.1) + (x / 2);
                        let base_v = (y / 2 * strides.2) + (x / 2);

                        let rgb_pixel = &mut self.buf[base_tgt..base_tgt + 3];

                        let y = i.y()[base_y] as f32;
                        let u = i.u()[base_u] as f32;
                        let v = i.v()[base_v] as f32;

                        rgb_pixel[2] = (y + 1.402 * (v - 128.0)) as u8;
                        rgb_pixel[1] = (y - 0.344 * (u - 128.0) - 0.714 * (v - 128.0)) as u8;
                        rgb_pixel[0] = (y + 1.772 * (u - 128.0)) as u8;
                    }
                }

                //                i.write_rgb8(&mut self.buf);
                println!(
                    "Took {} ms to write into rgb",
                    Instant::now().duration_since(b).as_millis()
                );
                self.buf.as_slice()
            });
        println!(
            "Took {} ms to decode image",
            Instant::now().duration_since(bb).as_millis()
        );
        a
    }
}

impl<T: 'static + ImageDecoder> Chain<T> for H264RGBDecoder {
    fn chain(self, other: T) -> ChainedDecoder {
        ChainedDecoder {
            a: Box::new(self),
            b: Box::new(other),
        }
    }
}

pub struct ChainedDecoder {
    a: Box<dyn ImageDecoder>,
    b: Box<dyn ImageDecoder>,
}

impl ImageDecoder for ChainedDecoder {
    fn decode(&mut self, data: &[u8]) -> Result<&[u8], DecoderError> {
        let b = Instant::now();
        let res = self.b.decode(self.a.decode(data)?);
        println!("Total decoding time => {}", b.elapsed().as_millis());
        res
    }
}

impl<T: 'static + ImageDecoder> Chain<T> for ChainedDecoder {
    fn chain(self, other: T) -> ChainedDecoder {
        ChainedDecoder {
            a: Box::new(self),
            b: Box::new(other),
        }
    }
}

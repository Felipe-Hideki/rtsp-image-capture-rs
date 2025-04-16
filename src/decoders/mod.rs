use std::{error::Error, time::Instant};

#[derive(Debug)]
pub enum DecoderError {
    InitFail(Box<dyn Error + Sync + Send>),
    DecodeFail(Box<dyn Error + Sync + Send>),
    NoImageDecoded,
    FieldOutOfBounds,
    NalOutOfBounds,
    IndexOutOfBounds(usize, Instant),
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

pub struct ChainedDecoder {
    a: Box<dyn ImageDecoder>,
    b: Box<dyn ImageDecoder>,
}

impl ChainedDecoder {
    pub fn new(a: Box<dyn ImageDecoder>, b: Box<dyn ImageDecoder>) -> ChainedDecoder {
        ChainedDecoder { a, b }
    }
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

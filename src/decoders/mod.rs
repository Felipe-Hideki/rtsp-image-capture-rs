pub mod avcc;

use avcc::AVCCDecoder;
use openh264::{
    decoder::{DecodedYUV, Decoder, DecoderConfig},
    OpenH264API,
};

#[derive(Debug)]
pub enum DecoderError {
    InitFail(openh264::Error),
    DecodeFail(openh264::Error),
    FieldOutOfBounds,
    NalOutofBounds,
}

pub struct H264Decoder {
    inner: Decoder,
    avccdecoder: AVCCDecoder,
}

impl H264Decoder {
    pub fn new(dbg: bool) -> Result<Self, DecoderError> {
        let decoder =
            Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().debug(dbg))
                .map_err(|e| DecoderError::InitFail(e))?;

        Ok(Self {
            inner: decoder,
            avccdecoder: AVCCDecoder::new(),
        })
    }

    pub fn decode_to_annex_b(&mut self, data: &[u8]) -> Result<&[u8], DecoderError> {
        self.avccdecoder.avcc_to_annex_b(data)
    }

    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedYUV<'_>>, DecoderError> {
        self.inner
            .decode(data)
            .map_err(|e| DecoderError::DecodeFail(e))
    }
}

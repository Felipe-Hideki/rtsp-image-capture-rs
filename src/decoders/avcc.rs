use super::DecoderError;

pub struct AVCCDecoder {
    buf: Vec<u8>,
}

impl AVCCDecoder {
    pub fn new() -> Self {
        return Self { buf: Vec::new() };
    }
    pub fn avcc_to_annex_b(&mut self, avcc_data: &[u8]) -> Result<&[u8], DecoderError> {
        self.buf.clear();
        let mut index = 0;
        let mut annex_b_size = 0;

        while index < avcc_data.len() {
            // Read the 4-byte size field
            if index + 4 > avcc_data.len() {
                return Err(DecoderError::FieldOutOfBounds);
            }

            let nal_size = u32::from_be_bytes([
                avcc_data[index],
                avcc_data[index + 1],
                avcc_data[index + 2],
                avcc_data[index + 3],
            ]) as usize;

            index += 4; // Skip the size field

            if index + nal_size > avcc_data.len() {
                return Err(DecoderError::NalOutofBounds);
            }

            // Extract the NAL unit
            let nal_unit = &avcc_data[index..index + nal_size];
            index += nal_size;

            // Prepend the Annex B start code (0x00000001)
            self.buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            self.buf.extend_from_slice(nal_unit);
            annex_b_size += nal_size + 4
        }

        Ok(&self.buf[..annex_b_size])
    }
}

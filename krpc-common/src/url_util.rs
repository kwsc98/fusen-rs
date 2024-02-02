use percent_encoding::{self, percent_decode_str, percent_encode, AsciiSet, CONTROLS};
const FRAGMENT: &AsciiSet = &CONTROLS.add(b':').add(b'/').add(b'&').add(b'?').add(b'=').add(b',');

pub fn decode_url(url: &str) -> crate::Result<String> {
    Ok(percent_decode_str(url).decode_utf8()?.to_string())
}
pub fn encode_url(url: &str) -> crate::Result<String> {
    Ok(percent_encode(url.as_bytes(),FRAGMENT).to_string())
}

use percent_encoding::{self, percent_decode_str, percent_encode, AsciiSet, CONTROLS};
const FRAGMENT: &AsciiSet = &CONTROLS
    .add(b':')
    .add(b'/')
    .add(b'&')
    .add(b'?')
    .add(b'=')
    .add(b',');

pub fn decode_url(url: &str) -> Result<String, String> {
    Ok(percent_decode_str(url)
        .decode_utf8()
        .map_err(|e|e.to_string())?.to_string())
}
pub fn encode_url(url: &str) -> String {
    percent_encode(url.as_bytes(), FRAGMENT).to_string()
}

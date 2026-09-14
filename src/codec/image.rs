use crate::ir::ImageSource;
use crate::Error;

pub(crate) fn decode_openai_image_url(url: String) -> Result<ImageSource, Error> {
    match url.strip_prefix("data:") {
        Some(rest) => parse_data_uri(rest),
        None => Ok(ImageSource::RemoteUrl(url.into_boxed_str())),
    }
}

pub(crate) fn encode_openai_image_url(source: &ImageSource) -> String {
    match source {
        ImageSource::RemoteUrl(url) => url.to_string(),
        ImageSource::Base64 { media_type, data } => {
            format!("data:{media_type};base64,{data}")
        }
    }
}

fn parse_data_uri(rest: &str) -> Result<ImageSource, Error> {
    let (metadata, data) = rest
        .split_once(',')
        .ok_or_else(|| Error::InvalidInput("image data URI missing comma".to_owned()))?;
    let (media_type, marker) = metadata
        .split_once(';')
        .ok_or_else(|| Error::InvalidInput("image data URI missing ;base64".to_owned()))?;

    if media_type.is_empty() {
        return Err(Error::InvalidInput(
            "image data URI missing media type".to_owned(),
        ));
    }
    if marker != "base64" {
        return Err(Error::InvalidInput(
            "image data URI must use base64".to_owned(),
        ));
    }
    if data.is_empty() || !is_base64(data) {
        return Err(Error::InvalidInput(
            "image data URI has invalid base64 payload".to_owned(),
        ));
    }

    Ok(ImageSource::Base64 {
        media_type: media_type.into(),
        data: data.into(),
    })
}

fn is_base64(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return false;
    }

    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return false;
    }

    let payload_len = bytes.len() - padding;
    bytes[..payload_len]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'+' || *byte == b'/')
        && bytes[payload_len..].iter().all(|byte| *byte == b'=')
}

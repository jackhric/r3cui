//! Kodi's artwork references (`image://<percent-encoded url>/`): the one encoder and decoder, and asking the source
//! for a smaller copy where it can make one.

use crate::wire::ArtRef;

/// Percent-encodes everything but unreserved characters, in lowercase hex as Kodi writes its own `image://` URLs
/// (they are its artwork cache keys, so the spelling matters).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02x}"));
        }
    }
    out
}

pub fn decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if let (b'%', Some(&h), Some(&l)) = (b[i], b.get(i + 1), b.get(i + 2)) {
            if let (Some(h), Some(l)) = (hex(h), hex(l)) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A smaller copy, `w` by `h`, of an artwork whose source can make one: Jellyfin and Emby resize on request, and
/// it still comes through Kodi's cache. Anything else is returned as it is.
pub fn sized(art: &ArtRef, w: u32, h: u32) -> ArtRef {
    let Some(inner) = art.0.strip_prefix("image://").map(|a| a.trim_end_matches('/')) else { return art.clone() };
    let url = decode(inner);
    if !url.contains("/Images/") || !url.contains("Format=original") {
        return art.clone();
    }
    let url = url.replace("Format=original", &format!("fillWidth={w}&fillHeight={h}&quality=85"));
    ArtRef(format!("image://{}/", encode(&url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding() {
        assert_eq!(encode("image://a b/"), "image%3a%2f%2fa%20b%2f");
        assert_eq!(decode("a%2Fb%2fc%"), "a/b/c%");
        assert_eq!(decode("%é"), "%é");
    }

    #[test]
    fn jellyfin_posters_come_small() {
        let art = ArtRef("image://https%3a%2f%2fmedia.example.com%2fItems%2fabc%2fImages%2fPrimary%2f0%3fFormat%3doriginal%26Tag%3d123/".into());
        assert_eq!(
            sized(&art, 136, 204).0,
            "image://https%3a%2f%2fmedia.example.com%2fItems%2fabc%2fImages%2fPrimary%2f0%3ffillWidth%3d136%26fillHeight%3d204%26quality%3d85%26Tag%3d123/"
        );
        let other = ArtRef("image://DefaultVideo.png/".into());
        assert_eq!(sized(&other, 136, 204), other);
    }
}

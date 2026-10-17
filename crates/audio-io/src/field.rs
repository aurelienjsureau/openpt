#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldInfo {
    pub project: String,
    pub scene: String,
    pub take: String,
    pub tape: String,
    pub note: String,
    pub circled: bool,
    pub tracks: Vec<String>,
}

impl FieldInfo {
    pub fn is_empty(&self) -> bool {
        *self == FieldInfo::default()
    }

    pub fn scene_take(&self) -> String {
        match (self.scene.is_empty(), self.take.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!("Sc {}", self.scene),
            (true, false) => format!("Tk {}", self.take),
            (false, false) => format!("Sc {} Tk {}", self.scene, self.take),
        }
    }
}

fn chunk<'a>(b: &'a [u8], want: &[u8; 4]) -> Option<&'a [u8]> {
    if b.len() < 12 || !(b.starts_with(b"RIFF") || b.starts_with(b"RF64")) || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        let body = pos + 8;
        if id == want {
            return b.get(body..body.saturating_add(size).min(b.len()));
        }
        if id == b"data" && size == u32::MAX as usize {
            return None;
        }
        pos = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

fn tag(xml: &str, name: &str, from: usize) -> Option<(String, usize)> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let s = xml.get(from..)?.find(&open)? + from + open.len();
    let e = xml.get(s..)?.find(&close)? + s;
    Some((unescape(xml[s..e].trim()), e + close.len()))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn from_ixml(xml: &str) -> FieldInfo {
    let get = |name: &str| tag(xml, name, 0).map(|t| t.0).unwrap_or_default();
    let mut info = FieldInfo {
        project: get("PROJECT"),
        scene: get("SCENE"),
        take: get("TAKE"),
        tape: get("TAPE"),
        note: get("NOTE"),
        circled: get("CIRCLED").eq_ignore_ascii_case("true"),
        tracks: Vec::new(),
    };
    let mut at = 0;
    while let Some((track, end)) = tag(xml, "TRACK", at) {
        at = end;
        let index = tag(&track, "CHANNEL_INDEX", 0).and_then(|t| t.0.parse::<usize>().ok()).unwrap_or(info.tracks.len() + 1);
        let name = tag(&track, "NAME", 0).map(|t| t.0).unwrap_or_default();
        if (1..=64).contains(&index) {
            if info.tracks.len() < index {
                info.tracks.resize(index, String::new());
            }
            info.tracks[index - 1] = name;
        }
    }
    info
}

fn from_bext(desc: &str) -> FieldInfo {
    let mut info = FieldInfo::default();
    for line in desc.split(['\r', '\n']) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "sSCENE" => info.scene = v,
            "sTAKE" => info.take = v,
            "sTAPE" => info.tape = v,
            "sPROJECT" => info.project = v,
            "sNOTE" => info.note = v,
            "sCIRCLED" => info.circled = v.eq_ignore_ascii_case("true"),
            k => {
                if let Some(n) = k.strip_prefix("sTRK").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=64).contains(n)) {
                    if info.tracks.len() < n {
                        info.tracks.resize(n, String::new());
                    }
                    info.tracks[n - 1] = v;
                }
            }
        }
    }
    info
}

pub fn field_info(bytes: &[u8]) -> Option<FieldInfo> {
    if let Some(x) = chunk(bytes, b"iXML") {
        let xml = String::from_utf8_lossy(x.split(|b| *b == 0).next().unwrap_or(x));
        let info = from_ixml(&xml);
        if !info.is_empty() {
            return Some(info);
        }
    }
    let bext = chunk(bytes, b"bext")?;
    let desc = String::from_utf8_lossy(bext.get(..256)?.split(|b| *b == 0).next().unwrap_or_default());
    Some(from_bext(&desc)).filter(|i| !i.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_with(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
        b.extend_from_slice(id);
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(body);
        if body.len() % 2 == 1 {
            b.push(0);
        }
        b
    }

    #[test]
    fn reads_ixml_scene_take_and_track_names() {
        let xml = "<BWFXML><PROJECT>Film</PROJECT><SCENE>12A</SCENE><TAKE>3</TAKE><CIRCLED>TRUE</CIRCLED>\
            <TRACK_LIST><TRACK><CHANNEL_INDEX>2</CHANNEL_INDEX><NAME>Lav &amp; Co</NAME></TRACK>\
            <TRACK><CHANNEL_INDEX>1</CHANNEL_INDEX><NAME>Boom</NAME></TRACK></TRACK_LIST></BWFXML>";
        let info = field_info(&wav_with(b"iXML", xml.as_bytes())).unwrap();
        assert_eq!((info.project.as_str(), info.scene.as_str(), info.take.as_str(), info.circled), ("Film", "12A", "3", true));
        assert_eq!(info.tracks, vec!["Boom".to_string(), "Lav & Co".to_string()]);
        assert_eq!(info.scene_take(), "Sc 12A Tk 3");
    }

    #[test]
    fn falls_back_to_bext_lines() {
        let mut bext = b"sSCENE=4\r\nsTAKE=7\r\nsTRK1=Boom\r\nsTRK3=Lav2\r\n".to_vec();
        bext.resize(602, 0);
        let info = field_info(&wav_with(b"bext", &bext)).unwrap();
        assert_eq!((info.scene.as_str(), info.take.as_str()), ("4", "7"));
        assert_eq!(info.tracks, vec!["Boom".to_string(), String::new(), "Lav2".to_string()]);
    }

    #[test]
    fn plain_files_have_none() {
        assert_eq!(field_info(&wav_with(b"junk", b"abcd")), None);
        assert_eq!(field_info(b"not a wav"), None);
    }
}

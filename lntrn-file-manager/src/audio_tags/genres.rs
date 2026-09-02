//! ID3v1 genre table — the numeric refs that ID3v2 `TCON` frames and the
//! ID3v1 trailer still use. Standard list 0–79 plus the common Winamp
//! extension up to 125.

const GENRES: &str = "Blues|Classic Rock|Country|Dance|Disco|Funk|Grunge|Hip-Hop|Jazz|Metal|\
New Age|Oldies|Other|Pop|R&B|Rap|Reggae|Rock|Techno|Industrial|Alternative|Ska|Death Metal|\
Pranks|Soundtrack|Euro-Techno|Ambient|Trip-Hop|Vocal|Jazz+Funk|Fusion|Trance|Classical|\
Instrumental|Acid|House|Game|Sound Clip|Gospel|Noise|Alternative Rock|Bass|Soul|Punk|Space|\
Meditative|Instrumental Pop|Instrumental Rock|Ethnic|Gothic|Darkwave|Techno-Industrial|\
Electronic|Pop-Folk|Eurodance|Dream|Southern Rock|Comedy|Cult|Gangsta|Top 40|Christian Rap|\
Pop/Funk|Jungle|Native American|Cabaret|New Wave|Psychedelic|Rave|Showtunes|Trailer|Lo-Fi|\
Tribal|Acid Punk|Acid Jazz|Polka|Retro|Musical|Rock & Roll|Hard Rock|Folk|Folk-Rock|\
National Folk|Swing|Fast Fusion|Bebop|Latin|Revival|Celtic|Bluegrass|Avantgarde|Gothic Rock|\
Progressive Rock|Psychedelic Rock|Symphonic Rock|Slow Rock|Big Band|Chorus|Easy Listening|\
Acoustic|Humour|Speech|Chanson|Opera|Chamber Music|Sonata|Symphony|Booty Bass|Primus|\
Porn Groove|Satire|Slow Jam|Club|Tango|Samba|Folklore|Ballad|Power Ballad|Rhythmic Soul|\
Freestyle|Duet|Punk Rock|Drum Solo|A Cappella|Euro-House|Dance Hall";

pub fn name(idx: usize) -> Option<&'static str> {
    GENRES.split('|').nth(idx)
}

pub fn index_of(name: &str) -> Option<u8> {
    let name = name.trim();
    GENRES
        .split('|')
        .position(|g| g.eq_ignore_ascii_case(name))
        .map(|i| i as u8)
}

/// Expand ID3v2 `TCON` shorthand — "(17)", "(17)Rock", "17", "(RX)" — into
/// readable text. Plain names pass through untouched.
pub fn resolve_tcon(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if let Ok(n) = raw.parse::<usize>() {
        return name(n).unwrap_or(raw).to_string();
    }
    let mut out: Vec<String> = Vec::new();
    let mut rest = raw;
    while let Some(r) = rest.strip_prefix('(') {
        if r.starts_with('(') {
            // "((" escapes a literal opening paren.
            rest = r;
            break;
        }
        let Some(close) = r.find(')') else { break };
        let code = &r[..close];
        rest = &r[close + 1..];
        let label = match code {
            "RX" => "Remix".to_string(),
            "CR" => "Cover".to_string(),
            c => c
                .parse::<usize>()
                .ok()
                .and_then(name)
                .map(|s| s.to_string())
                .unwrap_or_else(|| c.to_string()),
        };
        out.push(label);
    }
    let rest = rest.trim();
    if !rest.is_empty() && !out.iter().any(|o| o == rest) {
        out.push(rest.to_string());
    }
    out.join(" / ")
}

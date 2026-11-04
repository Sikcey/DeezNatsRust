//! Provider formats. Timings are read from the source, never estimated from text.
use super::{LyricLine, Syllable};
use serde_json::Value;

pub(super) fn lyricsfile(text: &str) -> Vec<LyricLine> {
    if text.len() > 2 * 1024 * 1024 {
        return Vec::new();
    }
    let Ok(value) = serde_yaml_ng::from_str::<Value>(text) else {
        return Vec::new();
    };
    value["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .take(2000)
        .filter_map(|line| {
            let text = line["text"].as_str()?.to_owned();
            let start = line["start_ms"].as_u64();
            let words = line["words"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|word| {
                    let start_ms = word["start_ms"].as_u64()?;
                    let end_ms = word["end_ms"].as_u64()?;
                    (end_ms > start_ms).then(|| Syllable {
                        start_ms,
                        end_ms,
                        text: word["text"].as_str().unwrap_or_default().into(),
                    })
                })
                .collect();
            Some(LyricLine {
                start_ms: start.unwrap_or(0),
                end_ms: line["end_ms"]
                    .as_u64()
                    .unwrap_or_else(|| start.map(|n| n.saturating_add(1)).unwrap_or(0)),
                text,
                syllables: words,
                translation: String::new(),
            })
        })
        .collect()
}

// NetEase YRC: [absolute line start,line duration](absolute word start,duration,flags)word...
pub(super) fn yrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in text.lines().take(2000) {
        let Some((header, mut rest)) = raw.strip_prefix('[').and_then(|s| s.split_once(']')) else {
            continue;
        };
        let Some((start, duration)) = header
            .split_once(',')
            .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
        else {
            continue;
        };
        let mut words = Vec::new();
        while let Some(after) = rest.strip_prefix('(') {
            let Some((header, tail)) = after.split_once(')') else {
                break;
            };
            let fields: Vec<_> = header.split(',').collect();
            if fields.len() != 3 {
                break;
            }
            let (Ok(ws), Ok(duration)) = (fields[0].parse::<u64>(), fields[1].parse::<u64>())
            else {
                break;
            };
            let next = tail.find('(').unwrap_or(tail.len());
            if duration > 0 {
                words.push(Syllable {
                    start_ms: ws,
                    end_ms: ws.saturating_add(duration),
                    text: tail[..next].into(),
                });
            }
            rest = &tail[next..];
        }
        if !words.is_empty() {
            lines.push(LyricLine {
                start_ms: start,
                end_ms: start.saturating_add(duration),
                text: words.iter().map(|w| w.text.as_str()).collect(),
                syllables: words,
                translation: String::new(),
            });
        }
    }
    lines
}

fn ttml_time(text: &str) -> Option<u64> {
    if let Some(ms) = text.strip_suffix("ms") {
        return ms.parse().ok();
    }
    if let Some(seconds) = text.strip_suffix('s') {
        return seconds_to_ms(seconds);
    }
    let fields: Vec<_> = text.split(':').collect();
    match fields.as_slice() {
        [seconds] => seconds_to_ms(seconds),
        [minutes, seconds] => minutes
            .parse::<u64>()
            .ok()?
            .checked_mul(60_000)?
            .checked_add(seconds_to_ms(seconds)?),
        [hours, minutes, seconds] => hours
            .parse::<u64>()
            .ok()?
            .checked_mul(3_600_000)?
            .checked_add(minutes.parse::<u64>().ok()?.checked_mul(60_000)?)?
            .checked_add(seconds_to_ms(seconds)?),
        _ => None,
    }
}
fn seconds_to_ms(text: &str) -> Option<u64> {
    let seconds = text.parse::<f64>().ok()?;
    (seconds.is_finite() && seconds >= 0.0 && seconds < 86400.0)
        .then(|| (seconds * 1000.0).round() as u64)
}
fn auxiliary(node: roxmltree::Node<'_, '_>) -> bool {
    node.ancestors().any(|parent| {
        parent.is_element()
            && parent.attributes().any(|attribute| {
                attribute.name() == "role"
                    && matches!(attribute.value(), "x-translation" | "x-roman" | "x-bg")
            })
    })
}

pub(super) fn ttml(text: &str) -> Vec<LyricLine> {
    if text.len() > 2 * 1024 * 1024 {
        return Vec::new();
    }
    let Ok(doc) = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 50_000,
            ..Default::default()
        },
    ) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("p")).take(2000) {
        if auxiliary(node) {
            continue;
        }
        let start = node.attribute("begin").and_then(ttml_time);
        let end = node.attribute("end").and_then(ttml_time).or_else(|| {
            node.attribute("dur")
                .and_then(ttml_time)
                .map(|dur| start.unwrap_or(0).saturating_add(dur))
        });
        let mut text = String::new();
        let mut words: Vec<Syllable> = Vec::new();
        for child in node
            .descendants()
            .filter(|child| child.is_text() && !auxiliary(*child))
        {
            let fragment = child.text().unwrap_or_default();
            text.push_str(fragment);
            // Nearest timed span owns this text, including nested emphasis spans.
            let timed = child
                .ancestors()
                .take_while(|parent| *parent != node)
                .find(|parent| parent.is_element() && parent.attribute("begin").is_some());
            if let Some(timed) = timed {
                if let (Some(ws), Some(we)) = (
                    timed.attribute("begin").and_then(ttml_time),
                    timed.attribute("end").and_then(ttml_time),
                ) {
                    if we > ws {
                        if let Some(previous) = words
                            .last_mut()
                            .filter(|word| word.start_ms == ws && word.end_ms == we)
                        {
                            previous.text.push_str(fragment);
                        } else {
                            words.push(Syllable {
                                start_ms: ws,
                                end_ms: we,
                                text: fragment.into(),
                            });
                        }
                    }
                }
            } else if fragment.chars().all(char::is_whitespace) {
                if let Some(previous) = words.last_mut() {
                    previous.text.push_str(fragment);
                }
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        lines.push(LyricLine {
            start_ms: start
                .or_else(|| words.first().map(|w| w.start_ms))
                .unwrap_or(0),
            end_ms: end
                .or_else(|| words.last().map(|w| w.end_ms))
                .unwrap_or_else(|| start.map(|s| s.saturating_add(1)).unwrap_or(0)),
            text: text.trim().into(),
            syllables: words,
            translation: node
                .descendants()
                .find(|n| {
                    n.is_element()
                        && n.attributes()
                            .any(|a| a.name() == "role" && a.value() == "x-translation")
                })
                .map(|n| {
                    n.descendants()
                        .filter_map(|n| n.is_text().then(|| n.text().unwrap_or_default()))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttml_accepts_apple_bare_seconds_before_minute_boundary() {
        let lines = ttml(
            r#"<tt><body><p begin="15.967" end="17.119"><span begin="15.967" end="17.119">First</span></p><p begin="58.045" end="59.167">Later</p><p begin="1:00.151" end="1:01.367">Next</p></body></tt>"#,
        );
        assert_eq!(
            lines.iter().map(|line| line.start_ms).collect::<Vec<_>>(),
            [15967, 58045, 60151]
        );
        assert_eq!(lines[0].syllables[0].end_ms, 17119);
        assert_eq!(lines[2].end_ms, 61367);
        assert_eq!(ttml_time("-1"), None);
        assert_eq!(ttml_time("NaN"), None);
    }
    #[test]
    fn word_formats_keep_real_word_times() {
        let yaml = "version: '1.0'\nlines:\n- text: Hello world\n  start_ms: 1000\n  end_ms: 2500\n  words:\n  - {text: 'Hello ', start_ms: 1000, end_ms: 1500}\n  - {text: world, start_ms: 1600, end_ms: 2500}\n";
        let lines = lyricsfile(yaml);
        assert_eq!(lines[0].syllables[1].start_ms, 1600);
        let lines = yrc("[1000,1500](1000,500,0)Hello (1600,900,0)world");
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].syllables[1].end_ms, 2500);
    }
    #[test]
    fn ttml_keeps_words_spaces_and_separates_translation() {
        let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><div><p begin="00:01.000" end="3s"><span begin="1s" end="1500ms">Hello</span> <span begin="1.6s" end="3s"><span>world &amp; you</span></span><span ttm:role="x-translation">Translation</span></p></div></body></tt>"#;
        let lines = ttml(xml);
        assert_eq!(lines[0].text, "Hello world & you");
        assert_eq!(lines[0].translation, "Translation");
        assert_eq!(lines[0].syllables.len(), 2);
        assert_eq!(lines[0].syllables[1].start_ms, 1600);
        assert!(ttml("<!DOCTYPE tt [<!ENTITY x 'entity'>]><tt><p>&x;</p></tt>").is_empty());
    }
}

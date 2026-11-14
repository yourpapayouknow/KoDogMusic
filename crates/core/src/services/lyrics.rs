use crate::domain::track::LyricLine;

/// 解析 LRC 文本格式歌词为毫秒时间轴列表
pub fn prslyrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    let mut offset = 0i64;
    for raw_line in text.lines() {
        let mut content = raw_line.trim();
        let mut times = Vec::new();
        while content.starts_with('[') {
            let Some(close) = content.find(']') else { break; };
            let tag = &content[1..close];
            content = &content[close + 1..];
            if let Some(value) = tag.strip_prefix("offset:").and_then(|value| value.parse::<i64>().ok()) { offset = value; }
            else if let Some((minutes, seconds)) = tag.split_once(':') {
                if let (Ok(minutes), Ok(seconds)) = (minutes.parse::<u64>(), seconds.parse::<f64>()) {
                    if seconds.is_finite() && (0.0..60.0).contains(&seconds) {
                        if let Some(time) = minutes.checked_mul(60_000).and_then(|time| time.checked_add((seconds * 1000.0).round() as u64)) { times.push(time); }
                    }
                }
            }
        }
        for time in times { lines.push(LyricLine { begin_ms: time, end_ms: None, text: content.trim().to_string() }); }
    }
    for line in &mut lines { line.begin_ms = line.begin_ms.saturating_add_signed(offset); }
    // 排序并计算单行结束时间
    lines.sort_by_key(|l| l.begin_ms);
    for i in 0..lines.len().saturating_sub(1) {
        let nxt_begin = lines[i + 1].begin_ms;
        lines[i].end_ms = Some(nxt_begin);
    }
    lines
}

#[cfg(test)]
mod tests {
    /// 验证重复时间标记、偏移、异常数值和最后一行。
    #[test]
    fn tst_lrc() {
        let lines = super::prslyrc("[offset:-500]\n[00:01.00][00:02.0]歌词\n[ar:歌手]\n[00:03.125]下一句\n[00:NaN]无效\n[18446744073709551615:00]溢出");
        assert_eq!(lines.iter().map(|line| line.begin_ms).collect::<Vec<_>>(), vec![500, 1500, 2625]);
        assert_eq!(lines[0].text, "歌词");
        assert_eq!(lines[0].end_ms, Some(1500));
        assert_eq!(lines[2].end_ms, None);
        assert!(super::prslyrc("[00:inf]无效\n[00:-1]无效\n[00:60]无效").is_empty());
    }
}

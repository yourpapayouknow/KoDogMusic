use crate::application::ports::StreamPort;
use crate::deps::serde_json::Value;
use crate::domain::error::{AppError, PlaybackError};
use crate::domain::ids::{ProviderId, TrackId};
use crate::domain::track::{AudioQuality, ResolvedSource, TrackSummary};
use crate::provider::kugou::client::KugouClient;

/// 酷狗音频流解析适配器
pub struct KugouStream {
    client: KugouClient,
}

impl KugouStream {
    /// 构造流适配器
    pub fn nw(client: KugouClient) -> Self {
        Self { client }
    }
}

impl StreamPort for KugouStream {
    /// 按实际选择的音质解析播放地址。
    fn rslv(&self, id: &TrackId, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
        if id.provider == ProviderId::Local {
            return Ok(ResolvedSource { url_or_local: id.raw_id.clone(), codec_hint: None });
        }
        medsrc(&self.client.trkurl(&id.raw_id, qlt)?, qlt)
    }

    /// 结合曲目多档哈希与 /privilege/lite 权限解析最佳音质播放地址。
    fn rslvtrk(&self, trk: &TrackSummary, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
        if trk.id.provider == ProviderId::Local {
            return Ok(ResolvedSource { url_or_local: trk.id.raw_id.clone(), codec_hint: None });
        }
        let mut hashes = vec![trk.id.raw_id.clone()];
        for h in trk.extra_hashes.values() {
            if !hashes.iter().any(|x| x.eq_ignore_ascii_case(h)) {
                hashes.push(h.clone());
            }
        }
        let mut candidates: Vec<(String, AudioQuality)> = Vec::new();
        if let Ok(prv) = self.client.prvlite(&hashes) {
            for hash in prvhshs(&prv, qlt.kgstr()) {
                candidates.push((hash, qlt));
            }
        }
        if let Some(h) = trk.extra_hashes.get(qlt.kgstr()) {
            if !candidates.iter().any(|(x, _)| x.eq_ignore_ascii_case(h)) {
                candidates.push((h.clone(), qlt));
            }
        }
        if !candidates.iter().any(|(x, _)| x.eq_ignore_ascii_case(&trk.id.raw_id)) {
            candidates.push((trk.id.raw_id.clone(), qlt));
        }

        let mut last_err: Option<AppError> = None;
        for (hash, quality) in candidates {
            match self.client.trkurl(&hash, quality) {
                Ok(root) => match medsrc(&root, quality) {
                    Ok(src) => return Ok(src),
                    Err(err) => last_err = Some(err),
                },
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or_else(|| PlaybackError::SourceExpired.into()))
    }

    /// 按音质链逐级降级解析播放地址。
    fn rslvply(&self, trk: &TrackSummary, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
        if trk.id.provider == ProviderId::Local {
            return self.rslvtrk(trk, qlt);
        }
        let chain: &[AudioQuality] = match qlt {
            AudioQuality::HiRes => &[AudioQuality::HiRes, AudioQuality::Lossless, AudioQuality::High, AudioQuality::Standard],
            AudioQuality::Lossless => &[AudioQuality::Lossless, AudioQuality::High, AudioQuality::Standard],
            AudioQuality::High => &[AudioQuality::High, AudioQuality::Standard],
            AudioQuality::Standard => &[AudioQuality::Standard],
        };
        let mut hashes = vec![trk.id.raw_id.clone()];
        for h in trk.extra_hashes.values() {
            if !hashes.iter().any(|x| x.eq_ignore_ascii_case(h)) {
                hashes.push(h.clone());
            }
        }
        let prv = self.client.prvlite(&hashes).ok();
        let mut candidates: Vec<(String, AudioQuality)> = Vec::new();
        for &tier in chain {
            if let Some(ref root) = prv {
                for hash in prvhshs(root, tier.kgstr()) {
                    if !candidates.iter().any(|(h, q)| *q == tier && h.eq_ignore_ascii_case(&hash)) {
                        candidates.push((hash, tier));
                    }
                }
            }
            if let Some(h) = trk.extra_hashes.get(tier.kgstr()) {
                if !candidates.iter().any(|(x, q)| *q == tier && x.eq_ignore_ascii_case(h)) {
                    candidates.push((h.clone(), tier));
                }
            }
            if !candidates.iter().any(|(x, q)| *q == tier && x.eq_ignore_ascii_case(&trk.id.raw_id)) {
                candidates.push((trk.id.raw_id.clone(), tier));
            }
        }
        let mut last_err: Option<AppError> = None;
        for (hash, quality) in candidates {
            match self.client.trkurl(&hash, quality) {
                Ok(root) => match medsrc(&root, quality) {
                    Ok(src) => return Ok(src),
                    Err(err) => last_err = Some(err),
                },
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or_else(|| PlaybackError::SourceExpired.into()))
    }
}

/// 从 /privilege/lite 响应中提取指定音质有权限的哈希列表。
fn prvhshs(root: &Value, qstr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(items) = root.get("data").and_then(Value::as_array) else { return out; };
    let mut collect = |entry: &Value| {
        if entry.get("level").and_then(Value::as_i64) == Some(0) { return; }
        if entry.get("quality").and_then(Value::as_str) == Some(qstr) {
            if let Some(h) = entry.get("hash").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                let low = h.to_ascii_lowercase();
                if !out.contains(&low) { out.push(low); }
            }
        }
    };
    for item in items {
        collect(item);
        if let Some(rel) = item.get("relate_goods").and_then(Value::as_array) {
            for entry in rel { collect(entry); }
        }
    }
    out
}

/// 核对实际格式和码率后提供音源，拒绝静默降质。
fn medsrc(root: &Value, quality: AudioQuality) -> Result<ResolvedSource, AppError> {
    let data = root.get("data").unwrap_or(&root);
    let data = data.as_array().and_then(|items| items.first()).unwrap_or(data);
    let url = data.get("url").and_then(|value|
        value.as_str().or_else(|| value.as_array().and_then(|items| items.first()).and_then(|value| value.as_str()))
    ).filter(|value| !value.is_empty()).ok_or(PlaybackError::SourceExpired)?;
    let parsed = crate::deps::url::Url::parse(url).map_err(|_| PlaybackError::SourceExpired)?;
    if !matches!(parsed.scheme(), "http" | "https") { return Err(PlaybackError::SourceExpired.into()); }
    let codec_hint = data.get("extName").and_then(|value| value.as_str()).map(|value| value.to_ascii_lowercase());
    if codec_hint.as_deref() == Some("mp4") {
        return Err(PlaybackError::Quality.into());
    }
    let bitrate = data.get("bitRate").and_then(|value| value.as_u64()).unwrap_or(0);
    let lossless = matches!(codec_hint.as_deref(), Some("flac" | "wav" | "ape" | "alac"));
    if (quality.islss() && !lossless) || (quality == AudioQuality::High && !lossless && bitrate > 0 && bitrate < 320_000) {
        return Err(PlaybackError::Quality.into());
    }
    Ok(ResolvedSource { url_or_local: url.to_string(), codec_hint })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 请求的无损或高品质不能被低码率返回假报成功。
    #[test]
    fn tst_medqlt() {
        let mut root = crate::deps::serde_json::json!({"url":["https://example.com/audio.mp3"],"extName":"mp3","bitRate":128000});
        assert!(medsrc(&root, AudioQuality::Standard).is_ok());
        assert!(medsrc(&root, AudioQuality::High).is_err());
        assert!(medsrc(&root, AudioQuality::Lossless).is_err());
        assert!(medsrc(&root, AudioQuality::HiRes).is_err());
        root["bitRate"] = 320000.into();
        assert!(medsrc(&root, AudioQuality::High).is_ok());
        root["extName"] = "flac".into(); root["bitRate"] = 0.into();
        assert_eq!(medsrc(&root, AudioQuality::Lossless).unwrap().codec_hint.as_deref(), Some("flac"));
        assert_eq!(medsrc(&root, AudioQuality::HiRes).unwrap().codec_hint.as_deref(), Some("flac"));
        root["extName"] = "mp4".into();
        assert!(medsrc(&root, AudioQuality::High).is_err());
        root["url"] = "file:///private".into();
        assert!(medsrc(&root, AudioQuality::Lossless).is_err());
    }
}

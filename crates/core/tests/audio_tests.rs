use kugou_core::audio::convert::Conv;
use kugou_core::audio::output::{OutConfig, OutFormat};

/// 验证流式重采样保留短音频和最后一块的时长。
#[test]
fn tst_rsmpend() {
    for (rate, frames) in [(22050, 400), (44100, 10000), (48000, 10000)] {
        let cfg = OutConfig { sample_rate: 48000, channels: 2, format: OutFormat::F32 };
        let mut conv = Conv::nw(rate, 1, &cfg).unwrap();
        let samples = vec![0.25; frames];
        let mut output = Vec::new();
        for chunk in samples.chunks(317) { conv.cnvr(chunk, &mut output).unwrap(); }
        conv.flsh(&mut output).unwrap();
        assert_eq!(output.len(), frames * 48000 / rate as usize * 2);
        assert!(output.iter().all(|x| x.is_finite()));
        let size = output.len();
        conv.flsh(&mut output).unwrap();
        assert_eq!(output.len(), size);
    }
}

/// 实际设备验证暂停后的定位不会被已满音频环阻塞。
#[test]
#[ignore = "需要Windows实际音频输出设备"]
fn tst_psseek() {
    use kugou_core::{audio::controller::AudioController, application::ports::PlaybackPort, domain::{track::ResolvedSource, playback::PlaybackState}};
    let root = std::env::temp_dir().join(format!("kodog-seek-{}", kugou_core::deps::uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("silence.wav");
    let size = 8000u32 * 20 * 2;
    let mut wave = b"RIFF".to_vec(); wave.extend((size + 36).to_le_bytes()); wave.extend(b"WAVEfmt "); wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes()); wave.extend(1u16.to_le_bytes()); wave.extend(8000u32.to_le_bytes()); wave.extend(16000u32.to_le_bytes());
    wave.extend(2u16.to_le_bytes()); wave.extend(16u16.to_le_bytes()); wave.extend(b"data"); wave.extend(size.to_le_bytes()); wave.resize(size as usize + 44, 0);
    std::fs::write(&path, wave).unwrap();
    let player = AudioController::nw();
    player.ply(ResolvedSource { url_or_local: path.display().to_string(), codec_hint: Some("wav".into()) }).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    player.ps();
    std::thread::sleep(std::time::Duration::from_millis(100));
    player.sk(5000);
    std::thread::sleep(std::time::Duration::from_millis(350));
    let progress = player.prgrss();
    player.stp();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(progress.state, PlaybackState::Paused);
    assert!(progress.position_ms >= 4800 && progress.position_ms <= 5100, "paused seek position: {}", progress.position_ms);
}

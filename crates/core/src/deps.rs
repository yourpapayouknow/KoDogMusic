// 外部依赖统一导出模块

// 编解码、加密与系统API。
pub use aes;
pub use base64;
pub use cipher;
pub use md5;
pub use rand;
pub use uuid;
pub use num_bigint::BigUint;
pub use zeroize::{Zeroize, Zeroizing};
pub use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN}};
pub mod win {
    pub use windows_sys::Win32::{Foundation::SYSTEMTIME, System::{SystemInformation::GetLocalTime, Com::CoTaskMemFree}, UI::Shell::{SHGetKnownFolderPath, FOLDERID_Downloads}, Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH}};
}

// 序列化与配置
pub use serde;
pub use serde_json;
pub use toml;

// 网络与异步
pub use reqwest;
pub use url;

// 数据库
pub use rusqlite;

// 错误与日志
pub use thiserror;
pub use tracing;

// 图像
pub use image;

// 音频解码。
pub use symphonia;
pub use symphonia::core::audio::GenericAudioBufferRef;
pub use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
pub use symphonia::core::codecs::CodecParameters;
pub use symphonia::core::errors::Error as SymphoniaError;
pub use symphonia::core::formats::probe::Hint;
pub use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
pub use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
pub use symphonia::core::meta::MetadataOptions;
pub use symphonia::core::units::{Time, TimeBase};
pub use symphonia::default::{get_codecs, get_probe};

// 音频输出 (CPAL)
pub use cpal;
pub use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
pub use cpal::{
    default_host, BufferSize, Device, FromSample, SampleFormat, SampleRate, SizedSample, Stream,
    StreamConfig, SupportedBufferSize, SupportedStreamConfig,
};

// 音频重采样。
pub use rubato::audioadapter_buffers::direct::InterleavedSlice;
pub use rubato::{Fft, FixedSync, Indexing, Resampler};

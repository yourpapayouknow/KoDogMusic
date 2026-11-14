use std::path::PathBuf;
use std::sync::Arc;
use crate::deps::*;

mod deps;

#[derive(Parser)]
#[command(name = "kgctl", about = "Kugou Lite CLI control & diagnostics", version = "0.1.1")]
struct Cli {
    /// 指定可选的数据目录（含 secrets.dat / library.db / config.toml）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 查看客户端当前播放状态与概要
    Status,
    /// 搜索单曲
    Search {
        query: String,
        #[arg(short, long, default_value_t = 10)]
        limit: u32,
    },
    /// 列出登录后 CoverFlow 封面集合
    Decks,
    /// 列出账号歌单或创建新歌单
    Playlist {
        #[arg(long)]
        create: Option<String>,
    },
    /// 探测曲目可用下载品质并可选执行下载
    Download {
        track_id: String,
        #[arg(long, default_value = "lossless")]
        quality: String,
        #[arg(long)]
        inspect_only: bool,
    },
    /// 播放指定曲目
    Play {
        track_id: String,
        /// 运行指定秒数后停止诊断播放。
        #[arg(long)]
        seconds: Option<u64>,
    },
    /// 暂停或恢复播放
    Toggle,
    /// 停止播放
    Stop,
}

/// 运行独立的核心诊断实例。
fn main() {
    let args = Cli::parse();

    let (storage, client) = if let Some(ref dir) = args.data_dir {
        let stg = Arc::new(DatabaseStorage::opn(dir.join("library.db"), dir.join("config.toml")).expect("初始化存储失败"));
        let cli = KugouClient::opn(dir.join("secrets.dat")).expect("初始化凭据失败");
        (stg, cli)
    } else {
        (Arc::new(DatabaseStorage::inmem().expect("初始化存储失败")), KugouClient::nw())
    };
    let catalog = Arc::new(KugouCatalog::nw(client.clone()));
    let stream = Arc::new(KugouStream::nw(client.clone()));
    let account = Arc::new(KugouAuth::nw(client.clone()));
    let library = Arc::new(KugouLibrary::nw(client));
    let audio = Arc::new(AudioController::nw());

    let mut coord = Coordinator::nw(
        catalog,
        account,
        library,
        stream,
        audio,
        storage,
    );
    let mut playback_failed = false;

    match args.cmd.unwrap_or(Commands::Status) {
        Commands::Status => {
            let ss = coord.ssht();
            println!("独立诊断实例播放状态: {:?}", ss.playback);
            println!("音量: {:.0}%", ss.volume * 100.0);
            println!("循环模式: {:?}", ss.repeat);
            println!("当前路由: {:?}", ss.route);
            if let Some(ref cur) = ss.current {
                println!("正在播放: {} - {}", cur.title, cur.artist_text);
            }
        }
        Commands::Search { query, limit } => {
            coord.hndlcmd(AppCommand::Search(query));
            let ss = coord.ssht();
            println!("搜索结果（前{}项）:", limit);
            for (idx, trk) in ss.search_results.iter().take(limit as usize).enumerate() {
                println!("{}. {} - {} [专辑: {}] ({})", idx + 1, trk.title, trk.artist_text, trk.album_name.as_deref().unwrap_or("-"), trk.id.raw_id);
            }
        }
        Commands::Decks => {
            coord.hndlcmd(AppCommand::LoadCoverFlow);
            for (idx, deck) in coord.ssht().decks.iter().enumerate() {
                println!("{}. [{:?}] {} (removable={})", idx, deck.kind, deck.title, deck.removable);
            }
        }
        Commands::Playlist { create } => {
            if let Some(name) = create {
                coord.hndlcmd(AppCommand::CreatePlaylist(name));
            } else {
                coord.hndlcmd(AppCommand::LoadCoverFlow);
            }
            for pl in &coord.ssht().user_playlists {
                println!("{} | {} ({}首)", pl.id, pl.name, pl.song_count);
            }
        }
        Commands::Download { track_id, quality, inspect_only } => {
            let id = TrackId::kg(track_id);
            coord.hndlcmd(AppCommand::QueueAdd(id.clone()));
            coord.hndlcmd(AppCommand::QueuePlay(0));
            if inspect_only {
                coord.hndlcmd(AppCommand::Stop);
                coord.hndlcmd(AppCommand::InspectDownload { scope: TargetScope::Single, search_index: None, whole_album: false });
                if let Some(ref prompt) = coord.ssht().download_prompt {
                    println!("可用下载品质: {:?}", prompt.available.iter().map(|q| q.qltlbl()).collect::<Vec<_>>());
                }
            } else {
                let q = match quality.as_str() {
                    "standard" | "128" => AudioQuality::Standard,
                    "high" | "320" => AudioQuality::High,
                    "hires" | "hi-res" => AudioQuality::HiRes,
                    _ => AudioQuality::Lossless,
                };
                coord.hndlcmd(AppCommand::SetQuality(q));
                coord.hndlcmd(AppCommand::Download(id));
            }
        }
        Commands::Play { track_id, seconds } => {
            let id = if std::path::Path::new(&track_id).is_file() {
                TrackId { provider: kugou_core::domain::ids::ProviderId::Local, raw_id: track_id }
            } else { TrackId::kg(track_id) };
            coord.hndlcmd(AppCommand::PlayTrack(id));
            let ss = coord.ssht();
            println!("已发起播放，当前状态: {:?}", ss.playback);
            if ss.error.is_none() {
                if let Some(seconds) = seconds {
                    std::thread::sleep(std::time::Duration::from_secs(seconds.min(3600)));
                } else {
                    println!("按回车停止诊断播放");
                    let mut line = String::new();
                    let _ = std::io::stdin().read_line(&mut line);
                }
                coord.tick();
                println!("实际播放状态: {:?}，进度{}ms", coord.ssht().playback, coord.ssht().position_ms);
                playback_failed = coord.ssht().playback == kugou_core::domain::playback::PlaybackState::Failed;
                coord.hndlcmd(AppCommand::Stop);
            }
        }
        Commands::Toggle => {
            coord.hndlcmd(AppCommand::TogglePlay);
            let ss = coord.ssht();
            println!("已切换播放，当前状态: {:?}", ss.playback);
        }
        Commands::Stop => {
            coord.hndlcmd(AppCommand::Stop);
            println!("已停止播放");
        }
    }
    if let Some(error) = &coord.ssht().error {
        eprintln!("{error}");
        std::process::exit(1);
    }
    if playback_failed {
        eprintln!("音源解码或设备输出失败");
        std::process::exit(1);
    }
}

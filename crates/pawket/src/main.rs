mod history;
mod jobs;
mod mcp;
mod server;
mod tools;

use clap::{Parser, Subcommand};
use pawket_core::{CookieSource, Engine};
use std::{net::IpAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "pawket",
    version,
    about = "One Rust media engine. Web, browser, terminal, and agents."
)]
struct Cli {
    /// Enable conversion of supported media using this local FFmpeg executable.
    /// Found on PATH automatically when not given.
    #[arg(long, global = true, env = "PAWKET_FFMPEG")]
    ffmpeg: Option<PathBuf>,
    /// Try YouTube videos that expose direct files in their public player.
    #[arg(long, global = true, env = "PAWKET_YOUTUBE_PREVIEW")]
    youtube_preview: bool,
    /// Use this local yt-dlp executable for other sites, HLS/DASH streams and playlists.
    /// Found on PATH automatically when not given.
    #[arg(long = "yt-dlp", global = true, env = "PAWKET_YTDLP")]
    ytdlp: Option<PathBuf>,
    /// Use your logins from this browser (chrome, firefox, safari, brave, edge,
    /// chromium, opera, vivaldi, whale; add :PROFILE for a specific profile).
    #[arg(
        long,
        global = true,
        env = "PAWKET_COOKIES_FROM_BROWSER",
        conflicts_with = "cookies"
    )]
    cookies_from_browser: Option<String>,
    /// Use logins from this Netscape-format cookies file. yt-dlp refreshes it in place.
    #[arg(long, global = true, env = "PAWKET_COOKIES")]
    cookies: Option<PathBuf>,
    /// Don't look for FFmpeg and yt-dlp on PATH; use only the paths given.
    #[arg(long, global = true, env = "PAWKET_NO_DETECT")]
    no_detect: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect the real downloadable formats available at a public HTTPS URL.
    Inspect { url: String },
    /// Save an original file or explicitly select an offered conversion.
    Get {
        url: String,
        #[arg(short, long, default_value = "downloads")]
        output: PathBuf,
        #[arg(long)]
        format: Option<String>,
    },
    /// Start the local website and authenticated HTTP API.
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        #[arg(long, default_value_t = 3987)]
        port: u16,
        #[arg(long, default_value = ".local/transfers")]
        output: PathBuf,
        /// Serve a web asset directory instead of the embedded interface.
        #[arg(long)]
        assets: Option<PathBuf>,
        /// Open the local Pawket page in the default browser.
        #[arg(long)]
        open: bool,
    },
    /// Run an MCP server over stdio. Stdout contains protocol messages only.
    Mcp {
        #[arg(short, long, default_value = "downloads")]
        output: PathBuf,
    },
    /// Report supported inputs and the boundaries of this release.
    Capabilities,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let engine = Engine::new()?.with_youtube_preview(cli.youtube_preview);
    let detect = |name: &str| (!cli.no_detect).then(|| tools::find(name)).flatten();
    let mut setup = Vec::new();
    // An explicit path must work. A program found on PATH is used only if
    // it passes the same checks; otherwise Pawket carries on without it.
    let engine = match (cli.ffmpeg.clone(), detect("ffmpeg")) {
        (Some(executable), _) => engine.with_ffmpeg(executable).await?,
        (None, Some(found)) => match engine.clone().with_ffmpeg(found.clone()).await {
            Ok(engine) => {
                setup.push(format!("FFmpeg: {}", found.display()));
                engine
            }
            Err(_) => {
                setup.push(format!(
                    "FFmpeg at {} lacks needed encoders; conversions are off",
                    found.display()
                ));
                engine
            }
        },
        (None, None) => engine,
    };
    let engine = match (cli.ytdlp.clone(), detect("yt-dlp")) {
        (Some(executable), _) => engine.with_ytdlp(executable).await?,
        (None, Some(found)) => match engine.clone().with_ytdlp(found.clone()).await {
            Ok(engine) => {
                setup.push(format!("yt-dlp: {}", found.display()));
                engine
            }
            Err(_) => {
                setup.push(format!(
                    "yt-dlp at {} didn't run; more sites are off",
                    found.display()
                ));
                engine
            }
        },
        (None, None) => engine,
    };
    let engine = match (cli.cookies_from_browser, cli.cookies) {
        (Some(browser), _) => engine.with_cookies(CookieSource::browser(&browser)?)?,
        (None, Some(file)) => engine.with_cookies(CookieSource::file(file).await?)?,
        (None, None) => engine,
    };
    match cli.command {
        Command::Inspect { url } => println!(
            "{}",
            serde_json::to_string_pretty(&engine.inspect(&url).await?)?
        ),
        Command::Get {
            url,
            output,
            format,
        } => {
            let result = engine.download(&url, &output, format.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Serve {
            host,
            port,
            output,
            assets,
            open,
        } => {
            for line in &setup {
                eprintln!("{line}");
            }
            server::run(engine, host, port, output, assets, open).await?
        }
        Command::Mcp { output } => mcp::run(engine, output).await?,
        Command::Capabilities => println!(
            "{}",
            serde_json::to_string_pretty(&server::capabilities(&engine))?
        ),
    }
    Ok(())
}

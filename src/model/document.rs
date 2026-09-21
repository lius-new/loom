//! Static file catalog: language, metadata and sample source code.
//!
//! The three buffers mirror the mockup's "Active Workspace" (WalletService.cs,
//! CombatEngine.rs, auth.ts). Adding a file = adding a `FileId` variant, a
//! `FileMeta` static, and an entry in `FILE_ORDER`.

use lgui::prelude::Color;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    CSharp,
    Rust,
    TypeScript,
}

impl Language {
    /// Short badge label shown in tabs / sidebar / palette.
    pub fn badge(self) -> &'static str {
        match self {
            Language::CSharp => "C#",
            Language::Rust => "RS",
            Language::TypeScript => "TS",
        }
    }

    /// Full language name for the status bar.
    pub fn long_name(self) -> &'static str {
        match self {
            Language::CSharp => "C#",
            Language::Rust => "Rust",
            Language::TypeScript => "TypeScript",
        }
    }

    pub fn badge_color(self) -> Color {
        match self {
            Language::CSharp => theme::PURPLE_400,
            Language::Rust => theme::ORANGE_400,
            Language::TypeScript => theme::BLUE_400,
        }
    }
}

/// Git status decoration for a file row/tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GitDot {
    Modified, // amber
    New,      // emerald
}

impl GitDot {
    pub fn color(self) -> Color {
        match self {
            GitDot::Modified => theme::AMBER_400,
            GitDot::New => theme::EMERALD_400,
        }
    }
}

/// Static metadata for one file.
pub struct FileMeta {
    pub name: &'static str,
    pub lang: Language,
    pub dir: &'static str,  // short directory label
    pub path: &'static str, // breadcrumb path
    pub code: &'static str,
    pub tests: &'static str, // "N tests passing"
    pub has_refs: bool,      // show "2 references" in the CodeLens ribbon
    pub git_dot: Option<GitDot>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FileId {
    Wallet,
    Combat,
    Auth,
}

pub const FILE_ORDER: [FileId; 3] = [FileId::Wallet, FileId::Combat, FileId::Auth];

/// A folder node in the workspace tree.
pub struct Folder {
    pub name: &'static str,
    pub subdirs: &'static [Folder],
    pub files: &'static [FileId],
}

pub const DIR_AUTH: Folder = Folder {
    name: "Auth",
    subdirs: &[],
    files: &[FileId::Auth],
};
pub const DIR_SERVICES: Folder = Folder {
    name: "Services",
    subdirs: &[],
    files: &[FileId::Wallet],
};
pub const DIR_SIMULATION: Folder = Folder {
    name: "Simulation",
    subdirs: &[],
    files: &[FileId::Combat],
};

/// Workspace root: everything lives under `src`.
pub const DIR_SRC: Folder = Folder {
    name: "src",
    subdirs: &[DIR_AUTH, DIR_SERVICES, DIR_SIMULATION],
    files: &[],
};

pub static WALLET: FileMeta = FileMeta {
    name: "WalletService.cs",
    lang: Language::CSharp,
    dir: "Services",
    path: "src > Services > WalletService.cs",
    code: r#"namespace GameServer.Services;

public sealed class PlayerWalletService : IWalletService
{
  private readonly IRedisDatabase _cache;
  private readonly ILogger<PlayerWalletService> _logger;

  public async Task<bool> TransferCreditsAsync(Guid sender, Guid receiver, decimal amount)
  {
    if (amount <= 0) throw new ArgumentOutOfRangeException(nameof(amount));
    if (senderBal < amount) return false;
    return await _cache.CommitTransferAsync(sender, receiver, amount);
  }
}"#,
    tests: "14 tests passing",
    has_refs: true,
    git_dot: Some(GitDot::Modified),
};

pub static COMBAT: FileMeta = FileMeta {
    name: "CombatEngine.rs",
    lang: Language::Rust,
    dir: "Simulation",
    path: "src > Simulation > CombatEngine.rs",
    code: r#"pub struct CombatEngine {
  tick_rate: u32,
  active_entities: Vec<EntityId>,
}

impl CombatEngine {
  pub fn calculate_damage_batch(&self, targets: &[Target]) -> Vec<f32> {
    // Parallel computation pass over active damage matrices
    targets.iter().map(|t| t.armor.calculate_mitigation(t.incoming)).collect()
  }
}"#,
    tests: "28 tests passing",
    has_refs: false,
    git_dot: None,
};

pub static AUTH: FileMeta = FileMeta {
    name: "auth.ts",
    lang: Language::TypeScript,
    dir: "Auth",
    path: "src > Auth > auth.ts",
    code: r#"import { SignJWT, jwtVerify } from 'jose';

export async function createSessionToken(userId: string): Promise<string> {
  return await new SignJWT({ sub: userId })
    .setProtectedHeader({ alg: 'HS256' })
    .setExpirationTime('2h')
    .sign(crypto.getRandomValues(new Uint8Array(32)));
}"#,
    tests: "8 tests passing",
    has_refs: false,
    git_dot: Some(GitDot::New),
};

pub fn meta(id: FileId) -> &'static FileMeta {
    match id {
        FileId::Wallet => &WALLET,
        FileId::Combat => &COMBAT,
        FileId::Auth => &AUTH,
    }
}

//! `clw completions <shell>` — generate shell-completion scripts to stdout.
//!
//! Purely client-side and offline: it reflects the derived `clap` command tree
//! (via [`clap::CommandFactory`] on [`crate::Cli`]) and asks `clap_complete` to
//! render a completion script for the requested shell. The script is written to
//! **stdout** so it can be redirected/sourced; a short install hint is printed to
//! **stderr** (so it never pollutes the script when piped to a file).
//!
//! No config, no network, no token — so this handler takes neither `ResolvedConfig`
//! nor `--json` (a completion script is not a JSON document).

use clap::{Args, CommandFactory as _};
use clap_complete::{generate, Shell};
use clw_types::ClwError;

use crate::Cli;

#[derive(Debug, Args)]
#[command(about = "Generate a shell-completion script (bash|zsh|fish|powershell|elvish) to stdout")]
pub struct CompletionsArgs {
    /// Which shell to generate completions for.
    #[arg(
        value_enum,
        help = "Target shell: bash, zsh, fish, powershell, or elvish"
    )]
    pub shell: Shell,
}

/// Render the completion script for `shell` into `out`.
///
/// Factored out of [`run`] so it is unit-testable without spawning the binary:
/// it writes the `clap_complete`-generated script for the derived [`Cli`] command
/// to the supplied writer. The binary name is fixed to `clw` (matching
/// `#[command(name = "clw")]`).
pub fn render(shell: Shell, out: &mut impl std::io::Write) {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    generate(shell, &mut cmd, bin_name, out);
}

/// Print a short, shell-appropriate install hint to stderr.
///
/// Kept off stdout so `clw completions bash > clw.bash` captures ONLY the script.
fn install_hint(shell: Shell) -> String {
    let common = match shell {
        Shell::Bash => {
            "# install: source it from ~/.bashrc, e.g.\n\
             #   clw completions bash > ~/.local/share/bash-completion/completions/clw"
        }
        Shell::Zsh => {
            "# install: write to a dir on your $fpath and run compinit, e.g.\n\
             #   clw completions zsh > \"${fpath[1]}/_clw\""
        }
        Shell::Fish => {
            "# install: drop it in fish's completions dir, e.g.\n\
             #   clw completions fish > ~/.config/fish/completions/clw.fish"
        }
        Shell::PowerShell => {
            "# install: append to your PowerShell profile, e.g.\n\
             #   clw completions powershell | Out-String | Invoke-Expression"
        }
        Shell::Elvish => {
            "# install: source it from rc.elv, e.g.\n\
             #   clw completions elvish > ~/.config/elvish/lib/clw.elv"
        }
        // `Shell` is non_exhaustive upstream; future shells fall back to a
        // generic note rather than failing to compile.
        _ => "# install: redirect this output to your shell's completion directory",
    };
    common.to_string()
}

// async to satisfy the uniform await-able subcommand dispatch signature (no async work here).
#[allow(clippy::unused_async)]
pub async fn run(args: CompletionsArgs) -> Result<(), ClwError> {
    let mut stdout = std::io::stdout();
    render(args.shell, &mut stdout);
    // Install hint to stderr so piping the script to a file stays clean.
    eprintln!("{}", install_hint(args.shell));
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Render a completion script for `shell` into a String (test helper).
    fn render_to_string(shell: Shell) -> String {
        let mut buf: Vec<u8> = Vec::new();
        render(shell, &mut buf);
        String::from_utf8(buf).expect("completion scripts are valid UTF-8")
    }

    #[test]
    fn each_shell_produces_nonempty_output_containing_clw() {
        for shell in [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Elvish,
        ] {
            let script = render_to_string(shell);
            assert!(
                !script.is_empty(),
                "{shell} completion script must be non-empty"
            );
            assert!(
                script.contains("clw"),
                "{shell} completion script must reference the `clw` binary name"
            );
        }
    }

    #[test]
    fn install_hint_is_nonempty_per_shell() {
        for shell in [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Elvish,
        ] {
            assert!(
                !install_hint(shell).is_empty(),
                "{shell} install hint must be non-empty"
            );
        }
    }
}

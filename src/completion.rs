//! Static shell completion scripts for the CLI.

use std::io::{self, Write};

use clap::{Command, ValueEnum};
use clap_complete::{Generator, shells::Bash};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum CompletionShell {
    Bash,
    Powershell,
}

pub(crate) fn write_script(
    shell: CompletionShell,
    command: Command,
    output: &mut dyn Write,
) -> io::Result<()> {
    let name = command.get_name().to_owned();
    let mut command = command.bin_name(&name);
    command.build();
    match shell {
        CompletionShell::Bash => {
            Bash.try_generate(&command, output)?;
            // Reuse Bash's serialized registration without assuming the generator's
            // function name or version-dependent completion options.
            writeln!(
                output,
                "\ncomplete -p {name} >/dev/null && eval \"$(complete -p {name}) {name}.exe\""
            )?;
        }
        CompletionShell::Powershell => write_powershell(&command, output)?,
    }
    output.flush()
}

fn write_powershell(command: &Command, output: &mut dyn Write) -> io::Result<()> {
    let name = command.get_name();
    writeln!(
        output,
        "Register-ArgumentCompleter -Native -CommandName {}, {} -ScriptBlock {{",
        ps_literal(name),
        ps_literal(&format!("{name}.exe"))
    )?;
    writeln!(
        output,
        "    param($wordToComplete, $commandAst, $cursorPosition)\n    $options = [ordered]@{{"
    )?;
    for arg in command.get_arguments().filter(|arg| !arg.is_hide_set()) {
        let Some(long) = arg.get_long() else { continue };
        let values = if arg.get_action().takes_values() {
            arg.get_value_parser().possible_values()
        } else {
            None
        };
        let values = values
            .into_iter()
            .flatten()
            .filter(|value| !value.is_hide_set())
            .map(|value| ps_literal(value.get_name()))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            output,
            "        {} = @({values})",
            ps_literal(&format!("--{long}"))
        )?;
    }
    writeln!(output, "    }}")?;
    output.write_all(include_bytes!("completion/powershell.ps1"))
}

fn ps_literal(text: &str) -> String {
    // PowerShell also recognizes typographic single quotes as delimiters.
    let mut literal = String::from("'");
    for ch in text.chars() {
        if matches!(ch, '\'' | '\u{2018}'..='\u{201b}') {
            literal.push(ch);
        }
        literal.push(ch);
    }
    literal.push('\'');
    literal
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::builder::{PossibleValue, PossibleValuesParser};

    #[test]
    fn powershell_does_not_offer_values_for_boolean_flags() {
        let mut output = Vec::new();
        write_script(
            CompletionShell::Powershell,
            crate::Args::command(),
            &mut output,
        )
        .unwrap();
        let script = String::from_utf8(output).unwrap();
        assert!(script.contains("'--force' = @()"));
        assert!(script.contains("'--dry-run' = @()"));
    }

    #[test]
    fn powershell_uses_visible_metadata_and_quotes_literals() {
        let mut command = Command::new("remove-except")
            .arg(clap::Arg::new("test-mode").long("test-mode").value_parser(
                PossibleValuesParser::new([
                    PossibleValue::new("plain"),
                    PossibleValue::new("it's ’literal’ $data"),
                    PossibleValue::new("hidden-value").hide(true),
                ]),
            ))
            .arg(
                clap::Arg::new("hidden-option")
                    .long("hidden-option")
                    .hide(true),
            );
        command.build();
        let mut output = Vec::new();
        write_powershell(&command, &mut output).unwrap();
        let script = String::from_utf8(output).unwrap();
        assert!(script.contains("'--test-mode' = @('plain', 'it''s ’’literal’’ $data')"));
        assert!(script.contains("'--help' = @()"));
        assert!(!script.contains("hidden-value"));
        assert!(!script.contains("hidden-option"));
    }
}

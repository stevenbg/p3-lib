//! Disassembler and linter for Patrician 3 mission scripts (`.p2m`): the letter-script
//! bytecode the interpreter at `0x004ECF64` runs.
//!
//! Layout: u16 command count, u16 variable count, one u32 file offset per command, the
//! commands, then the string pool. Every command's start is in the offset table, so a
//! command's length is known without knowing the command - the disassembler flags any
//! decoded length that disagrees with it, which is how new opcode lengths get established.
use std::{collections::BTreeMap, fs, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Disassemble scripts: one line per command, then the string pool.
    Dis {
        /// Files or glob patterns, e.g. "unpacked/*/missions_addon/*.p2m"
        #[arg(required = true)]
        inputs: Vec<String>,
    },
    /// Report create-letter commands whose town variable never receives a town.
    Lint {
        /// Files or glob patterns
        #[arg(required = true)]
        inputs: Vec<String>,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();
    let (inputs, lint) = match &args.command {
        Command::Dis { inputs } => (inputs, false),
        Command::Lint { inputs } => (inputs, true),
    };
    let files = expand(inputs);
    if files.is_empty() {
        eprintln!("no files matched");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    if lint {
        println!("checked {} files", files.len());
    }
    for path in &files {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        let script = match Script::parse(&data) {
            Ok(script) => script,
            Err(why) => {
                eprintln!("{}: {why}", path.display());
                failed = true;
                continue;
            }
        };
        if lint {
            lint_script(path, &script);
        } else {
            disassemble(path, &script);
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn expand(inputs: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for input in inputs {
        match glob::glob(input) {
            Ok(paths) => {
                let mut matched: Vec<PathBuf> = paths.flatten().filter(|p| p.is_file()).collect();
                matched.sort();
                if matched.is_empty() {
                    files.push(PathBuf::from(input));
                } else {
                    files.extend(matched);
                }
            }
            Err(_) => files.push(PathBuf::from(input)),
        }
    }
    files
}

/// A parsed script: the header, the command offsets and the raw bytes.
struct Script<'a> {
    data: &'a [u8],
    variables: u16,
    offsets: Vec<usize>,
}

impl<'a> Script<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        if data.len() < 4 {
            return Err("shorter than the header".into());
        }
        let count = u16::from_le_bytes([data[0], data[1]]) as usize;
        let variables = u16::from_le_bytes([data[2], data[3]]);
        if data.len() < 4 + 4 * count {
            return Err(format!("offset table for {count} commands does not fit"));
        }
        let offsets: Vec<usize> = (0..count).map(|i| u32::from_le_bytes(data[4 + 4 * i..8 + 4 * i].try_into().unwrap()) as usize).collect();
        match offsets.first() {
            Some(&first) if first == 4 + 4 * count => {}
            Some(_) => return Err("offsets[0] is not right after the offset table".into()),
            None => return Err("no commands".into()),
        }
        if offsets.iter().any(|&o| o >= data.len()) {
            return Err("an offset points past the end of the file".into());
        }
        Ok(Self { data, variables, offsets })
    }

    fn count(&self) -> usize {
        self.offsets.len()
    }

    /// The length the offset table gives a command; `None` for the last one.
    fn table_length(&self, index: usize) -> Option<usize> {
        self.offsets.get(index + 1).map(|next| next - self.offsets[index])
    }

    fn byte(&self, offset: usize, k: usize) -> u8 {
        self.data.get(offset + k).copied().unwrap_or(0)
    }
    fn word(&self, offset: usize, k: usize) -> u16 {
        u16::from_le_bytes([self.byte(offset, k), self.byte(offset, k + 1)])
    }
    fn dword(&self, offset: usize, k: usize) -> u32 {
        u32::from_le_bytes([self.byte(offset, k), self.byte(offset, k + 1), self.byte(offset, k + 2), self.byte(offset, k + 3)])
    }
}

/// A decoded command: its length, its text, and a note.
struct Decoded {
    length: Option<usize>,
    text: String,
    note: &'static str,
}

fn v(n: u8) -> String {
    format!("v{n}")
}

/// The known opcodes. The gitbook's opcode table (File Formats / Mission Scripts) is the
/// reference; anything not here prints as `???` with its bytes.
fn decode(s: &Script, off: usize) -> Decoded {
    let op = s.byte(off, 0);
    let b = |k| s.byte(off, k);
    let w = |k| s.word(off, k);
    let d = |length: usize, text: String, note: &'static str| Decoded { length: Some(length), text, note };
    match op {
        0x00 => d(2, format!("wait_until {}", v(b(1))), "sleep until that absolute time; abort script if it is >256 ticks past"),
        0x05 => d(2, format!("{} = now", v(b(1))), "game time in ticks (game_world+0x14)"),
        0x06 => d(3, format!("{} = now + {} days", v(b(2)), v(b(1))), "days = ticks*256"),
        0x07 => d(3, format!("{} = home_town_of_merchant({})", v(b(2)), v(b(1))), ""),
        0x09 => d(3, format!("{} = rand() % {}", v(b(2)), v(b(1))), ""),
        0x0A => d(6, format!("{} = {}", v(b(1)), s.dword(off, 2) as i32), ""),
        0x0B => d(4, format!("{} = {} + {}", v(b(3)), v(b(1)), v(b(2))), ""),
        0x0C => d(4, format!("{} = {} - {}", v(b(3)), v(b(1)), v(b(2))), ""),
        0x0D => d(4, format!("{} = {} * {}", v(b(3)), v(b(1)), v(b(2))), ""),
        0x0E => d(4, format!("{} = {} / {}", v(b(3)), v(b(1)), v(b(2))), ""),
        0x17..=0x1C => {
            let cmp = ["<", "<=", "==", "!=", ">=", ">"][(op - 0x17) as usize];
            d(7, format!("if {} {cmp} {} -> {} else -> {}", v(b(1)), v(b(2)), w(3), w(5)), "")
        }
        0x1D => d(3, format!("goto {}", w(1)), ""),
        0x1E => d(3, format!("{} = town_of_ship({})", v(b(2)), v(b(1))), "-1 unless docked (status <0xF, not 2/3)"),
        0x21 => d(2, format!("{} = random town", v(b(1))), ""),
        0x23 => d(3, format!("{} = random first name (seed {})", v(b(2)), v(b(1))), "printed by %V"),
        0x24 => d(2, format!("{} = random surname", v(b(1))), "printed by %N"),
        0x28 => d(3, format!("{} = owner_of_ship({})", v(b(2)), v(b(1))), "merchant count if none"),
        0x29 => d(6, format!("route_towns({}, {}, {}, {}, {})", v(b(1)), v(b(2)), v(b(3)), v(b(4)), v(b(5))), "picker through 0x00533210; writes v1..v4"),
        0x2A => d(4, format!("{} = ship_in_town({}, {})", v(b(3)), v(b(2)), v(b(1))), ""),
        0x34 => d(3, format!("{} = citizens_of_town({})", v(b(2)), v(b(1))), "town+0x2D4, handler 0x004F0A27"),
        0x39 => d(2, format!("{} = human_players", v(b(1))), "merchants whose mailbox gate (+0x8) is 0"),
        0x3A => d(3, format!("ship_reservation({}, {})", v(b(1)), v(b(2))), "flag!=0 locks ship+0x137, 0 releases"),
        0x43 => d(2, format!("{} = towns_count", v(b(1))), ""),
        0x44 => d(3, format!("{} = random_office_town({})", v(b(2)), v(b(1))), "a town the merchant has an office in"),
        0x56 => d(2, format!("{} = council_town", v(b(1))), "home town of merchant [0x6DFC14]"),
        0x57 => d(4, format!("{} = best_ship_of({}) in town {}", v(b(3)), v(b(1)), v(b(2))), "docked, unflagged; -1 if none"),
        0x58 => d(3, format!("{} = v[{}]", v(b(2)), v(b(1))), "indirect load"),
        0x59 => d(3, format!("v[{}] = {}", v(b(1)), v(b(2))), "indirect store"),
        0x61 => d(3, format!("merchant_word16_add({}, {})", v(b(1)), v(b(2))), "saturating add to merchant+0x16 via 0x004F36E0"),
        0x64 => d(2, format!("flag_ai_ships({}%)", v(b(1))), "sets ship+0x136 bit 0x20 on that share of every AI merchant's ships; handler 0x004F25E8"),
        0x65 => d(3, format!("{} = town_index_of_id({})", v(b(2)), v(b(1))), "-1 when out of range; handler 0x004F26C0"),
        0x6F => d(7, format!("pick_towns({}, {}, {}, {}, {}, {})", v(b(1)), v(b(2)), v(b(3)), v(b(4)), v(b(5)), v(b(6))), "writes v1..v4"),
        0xEC => d(2, format!("start_plague({})", v(b(1))), "operation 0x7C -> scheduled task 0x1C; refused if the town already has flag 0x8; handler 0x004EE7D0"),
        0xF1 => d(4, format!("{} = cargo({}, {})", v(b(3)), v(b(1)), v(b(2))), "positive amount loads (0x00518640), negative unloads (0x005186D0)"),
        0xF7 => d(
            12,
            format!(
                "letter kind={} (type {:#x}) town={} d=[{},{},{},{},{}] text={:#06x}",
                b(1),
                0x3c + b(1) as u32,
                v(b(2)),
                b(3),
                b(4),
                b(5),
                b(6),
                b(7),
                s.dword(off, 8)
            ),
            "",
        ),
        0xF8 => d(3, format!("pay({}, {})", v(b(1)), v(b(2))), "merchant money +0x0, book to +0x4B8/+0x4BC"),
        0xFD => d(4, format!("sleep {} ticks, resume at {}", v(b(1)), w(2)), ""),
        0xFE => d(2, format!("restart after {} days", v(b(1))), "PC back to 0"),
        0xFF => d(1, "end".into(), "frees the script"),
        _ => Decoded { length: None, text: format!("op {op:#04x} ???"), note: "" },
    }
}

fn disassemble(path: &PathBuf, s: &Script) {
    println!(
        "{}\n{} bytes, {} commands, {} variables, offsets[0]={:#x}\n",
        path.display(),
        s.data.len(),
        s.count(),
        s.variables,
        s.offsets[0]
    );
    let mut strings_start = s.data.len();
    for i in 0..s.count() {
        let off = s.offsets[i];
        let table = s.table_length(i);
        let decoded = decode(s, off);
        let length = decoded.length.or(table).unwrap_or(1);
        let raw: Vec<String> = s.data[off..(off + length).min(s.data.len())].iter().map(|b| format!("{b:02x}")).collect();
        let flag = match (table, decoded.length) {
            (Some(t), Some(l)) if t != l => format!("  << len {t} != {l}"),
            _ => String::new(),
        };
        let note = if decoded.note.is_empty() { String::new() } else { format!("   ; {}", decoded.note) };
        println!("{i:3}  {off:#06x}  {:<36} {}{note}{flag}", raw.join(" "), decoded.text);
        if i + 1 == s.count() {
            strings_start = off + length;
        }
    }
    println!("\n--- string pool ---");
    let mut pos = strings_start;
    while pos < s.data.len() {
        let end = s.data[pos..].iter().position(|&b| b == 0).map(|n| pos + n).unwrap_or(s.data.len());
        let text: String = s.data[pos..end].iter().map(|&b| b as char).collect();
        println!("{pos:#06x} ({:4})  {text}", text.len());
        pos = end + 1;
    }
}

/// What kind of value a command writes into which operand: the linter's knowledge.
/// `None` for the operand index means "any variable" (an indirect store).
const WRITERS: &[(u8, usize, &[(Option<usize>, &str)])] = &[
    (0x05, 2, &[(Some(1), "time")]),
    (0x07, 3, &[(Some(2), "TOWN")]),
    (0x29, 6, &[(Some(1), "TOWN"), (Some(2), "TOWN"), (Some(3), "TOWN"), (Some(4), "TOWN")]),
    (0x44, 3, &[(Some(2), "TOWN")]),
    (0x06, 3, &[(Some(2), "time")]),
    (0x09, 3, &[(Some(2), "rand")]),
    (0x0A, 6, &[(Some(1), "immediate")]),
    (0x0B, 4, &[(Some(3), "arith")]),
    (0x0C, 4, &[(Some(3), "arith")]),
    (0x0D, 4, &[(Some(3), "arith")]),
    (0x0E, 4, &[(Some(3), "arith")]),
    (0x1E, 3, &[(Some(2), "TOWN")]),
    (0x21, 2, &[(Some(1), "TOWN")]),
    (0x23, 3, &[(Some(2), "name")]),
    (0x24, 2, &[(Some(1), "name")]),
    (0x28, 3, &[(Some(2), "merchant")]),
    (0x2A, 4, &[(Some(3), "bool")]),
    (0x39, 2, &[(Some(1), "count")]),
    (0x3A, 3, &[]),
    (0x43, 2, &[(Some(1), "count")]),
    (0x53, 4, &[(Some(3), "distance")]),
    (0x56, 2, &[(Some(1), "TOWN")]),
    (0x57, 4, &[(Some(3), "ship")]),
    (0x58, 3, &[(Some(2), "indirect")]),
    (0x59, 3, &[(None, "indirect")]),
    (0x6F, 7, &[(Some(1), "TOWN"), (Some(2), "TOWN"), (Some(3), "TOWN"), (Some(4), "TOWN")]),
    (0xF1, 4, &[(Some(3), "cargo")]),
    (0xF8, 3, &[]),
    (0xFD, 4, &[]),
    (0xFE, 2, &[]),
    (0x17, 7, &[]),
    (0x18, 7, &[]),
    (0x19, 7, &[]),
    (0x1A, 7, &[]),
    (0x1B, 7, &[]),
    (0x1C, 7, &[]),
    (0x1D, 3, &[]),
    (0x00, 2, &[]),
    (0xFF, 1, &[]),
    (0xF7, 12, &[]),
];
/// Kinds that may legitimately hold a town.
const TOWNISH: &[&str] = &["TOWN", "immediate", "indirect", "unknown"];

fn lint_script(path: &PathBuf, s: &Script) {
    let variables = s.variables as usize;
    // variable -> (command index, opcode, kind) for every write
    let mut writers: BTreeMap<usize, Vec<(usize, u8, &str)>> = BTreeMap::new();
    for (i, &off) in s.offsets.iter().enumerate() {
        let op = s.byte(off, 0);
        match WRITERS.iter().find(|(o, _, _)| *o == op) {
            Some((_, _, targets)) => {
                for &(operand, kind) in targets.iter() {
                    match operand {
                        None => {
                            for var in 0..variables {
                                writers.entry(var).or_default().push((i, op, kind));
                            }
                        }
                        Some(k) => writers.entry(s.byte(off, k) as usize).or_default().push((i, op, kind)),
                    }
                }
            }
            None => {
                // unknown command: any operand that looks like a variable may be written
                let length = s.table_length(i).unwrap_or(1);
                for k in 1..length {
                    let var = s.byte(off, k) as usize;
                    if var < variables {
                        writers.entry(var).or_default().push((i, op, "unknown"));
                    }
                }
            }
        }
    }

    let mut findings = Vec::new();
    for (i, &off) in s.offsets.iter().enumerate() {
        if s.byte(off, 0) != 0xF7 {
            continue;
        }
        let town = s.byte(off, 2) as usize;
        if town >= variables {
            findings.push((i, town, "OUT OF RANGE", Vec::new()));
            continue;
        }
        let w = writers.get(&town).cloned().unwrap_or_default();
        if w.is_empty() {
            findings.push((i, town, "never written", w));
        } else if !w.iter().any(|(_, _, kind)| TOWNISH.contains(kind)) {
            findings.push((i, town, "no town-shaped write", w));
        }
    }
    if findings.is_empty() {
        return;
    }
    println!(
        "\n{}  ({} commands, {} variables)",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        s.count(),
        variables
    );
    for (i, town, why, w) in findings {
        println!("  command {i:3}: letter town = v{town}  <- {why}");
        for (ci, op, kind) in w {
            println!("      written at command {ci:3} by {op:02X} ({kind})");
        }
    }
}

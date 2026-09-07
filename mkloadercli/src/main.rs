//! Builds `Patrician3_modloader.exe` from the player's own `Patrician3.exe`.
//!
//! The launcher is the game's executable with one import added, `p3_modloader.dll`, so
//! that DLL's `DllMain` runs before any game code. Being the game's executable, it is not
//! something this project ships; this tool rebuilds it, byte for byte, from the copy the
//! player already owns:
//!
//! - a new section `.mod` is appended, holding a copy of the import directory plus one
//!   descriptor for `p3_modloader` with an empty thunk list - Windows loads a DLL named
//!   by a descriptor even when nothing is imported from it;
//! - the import data directory is pointed at that copy;
//! - the section count and `SizeOfImage` grow accordingly.
//!
//! The output for the v1.1 GOG executable is checked against the SHA-256 of the launcher
//! the project has always used, so a result that differs is refused rather than written.
//! Every other byte of the game, the checksum field included, is left as it is; Windows
//! does not verify an executable's checksum.
use std::{fs, path::PathBuf, process::ExitCode};

use clap::Parser;
use sha2::{Digest, Sha256};

/// The v1.1 GOG `Patrician3.exe` (2,961,408 bytes), the only input the recipe is verified for.
const INPUT_SHA256: &str = "fe436dc5f8addc4d3aa437eb48a8bc6415d9ba5577c1027d06db9bb915eddece";
/// The launcher built from it (2,965,504 bytes).
const OUTPUT_SHA256: &str = "2f960c4358df9b2a1c531bd842bf670c2ad6d1c5e6794869414708f31137bc08";

const SECTION_NAME: [u8; 8] = *b".mod\0\0\0\0";
/// The section's virtual size, as the launcher has it; its file data is one aligned page.
const SECTION_VIRTUAL_SIZE: u32 = 0x1_0000;
const SECTION_RAW_SIZE: u32 = 0x1000;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`.
const SECTION_CHARACTERISTICS: u32 = 0xC000_0040;
/// The DLL name as the launcher spells it: the loader reads up to the first NUL and
/// supplies the extension itself, so the bytes after it are inert.
const DLL_NAME: &[u8] = b"p3_modloader\0\0.dll\0";
/// Values the launcher carries in every section header's relocation and line-number
/// fields, which the loader ignores for an image. Written so the output matches it.
const STAMP_RELOCATIONS: u32 = 1;
const STAMP_LINENUMBERS: u32 = 2;
const STAMP_RELOCATION_COUNT: u16 = 3;
const STAMP_LINENUMBER_COUNT: u16 = 4;

const DESCRIPTOR_SIZE: usize = 20;
const SECTION_HEADER_SIZE: usize = 40;

const GAME_EXE: &str = "Patrician3.exe";
const LAUNCHER_EXE: &str = "Patrician3_modloader.exe";

#[derive(Parser, Debug)]
#[command(author, version, about = "Build Patrician3_modloader.exe from Patrician3.exe", long_about = None)]
struct Args {
    /// The game's Patrician3.exe (v1.1); default: the one in this program's own folder, so
    /// the program can be dropped into the game folder and double-clicked.
    #[arg(short, long)]
    input: Option<PathBuf>,
    /// Where to write the launcher; default: Patrician3_modloader.exe beside the input.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Patch an executable whose hash is not the known v1.1 one. The output then cannot be
    /// verified against the known launcher.
    #[arg(long)]
    force: bool,
}

fn main() -> ExitCode {
    // Started by a double-click there are no arguments and the console closes with the
    // program, so hold it until the result has been read.
    let interactive = std::env::args_os().len() == 1;
    let args = Args::parse();
    let code = match resolve_input(&args).and_then(|input| {
        let output = args.output.clone().unwrap_or_else(|| input.with_file_name(LAUNCHER_EXE));
        run(&args, &input, &output)
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    };
    if interactive {
        println!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    code
}

/// The `--input`, or `Patrician3.exe` beside this program.
fn resolve_input(args: &Args) -> Result<PathBuf, String> {
    if let Some(input) = &args.input {
        return Ok(input.clone());
    }
    let own = std::env::current_exe().map_err(|e| format!("cannot locate this program: {e}"))?;
    let folder = own.parent().ok_or("this program has no parent folder")?;
    let input = folder.join(GAME_EXE);
    if !input.exists() {
        return Err(format!(
            "no {GAME_EXE} in {} - put this program in the game folder, or pass --input",
            folder.display()
        ));
    }
    Ok(input)
}

fn run(args: &Args, input: &PathBuf, output: &PathBuf) -> Result<(), String> {
    let image = fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let input_hash = sha256(&image);
    let known_input = input_hash == INPUT_SHA256;
    if !known_input && !args.force {
        return Err(format!(
            "{} is not the v1.1 {GAME_EXE} (sha256 {input_hash}, expected {INPUT_SHA256}); pass --force to patch it anyway",
            input.display()
        ));
    }

    let patched = patch(&image)?;
    let output_hash = sha256(&patched);
    if known_input && output_hash != OUTPUT_SHA256 {
        return Err(format!(
            "the result (sha256 {output_hash}) does not match the known launcher ({OUTPUT_SHA256}); nothing written"
        ));
    }
    fs::write(output, &patched).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    println!("wrote {} ({} bytes)", output.display(), patched.len());
    println!(
        "sha256 {output_hash}{}",
        if known_input {
            " - matches the known launcher"
        } else {
            " - unverified input"
        }
    );
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn u16_at(b: &[u8], off: usize) -> Result<u16, String> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| format!("truncated at {off:#x}"))
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("truncated at {off:#x}"))
}

fn put_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn align_up(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_pointer: u32,
    raw_size: u32,
}

/// The whole recipe, on a copy of the image.
fn patch(image: &[u8]) -> Result<Vec<u8>, String> {
    if image.get(0..2) != Some(b"MZ") {
        return Err("not an MZ executable".into());
    }
    let pe = u32_at(image, 0x3c)? as usize;
    if image.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("PE signature not found".into());
    }
    let section_count = u16_at(image, pe + 6)? as usize;
    let optional_size = u16_at(image, pe + 20)? as usize;
    let opt = pe + 24;
    if u16_at(image, opt)? != 0x10b {
        return Err("not a PE32 (32-bit) image".into());
    }
    let section_alignment = u32_at(image, opt + 0x20)?;
    let file_alignment = u32_at(image, opt + 0x24)?;
    let size_of_image = u32_at(image, opt + 0x38)?;
    let size_of_headers = u32_at(image, opt + 0x3c)? as usize;
    let import_dir = opt + 0x60 + 8;
    let import_rva = u32_at(image, import_dir)?;

    let table = pe + 24 + optional_size;
    if table + (section_count + 1) * SECTION_HEADER_SIZE > size_of_headers {
        return Err("no room in the headers for another section".into());
    }
    let mut sections = Vec::with_capacity(section_count);
    for i in 0..section_count {
        let h = table + i * SECTION_HEADER_SIZE;
        sections.push(Section {
            virtual_size: u32_at(image, h + 8)?,
            virtual_address: u32_at(image, h + 12)?,
            raw_size: u32_at(image, h + 16)?,
            raw_pointer: u32_at(image, h + 20)?,
        });
    }
    let rva_to_offset = |rva: u32| -> Result<usize, String> {
        sections
            .iter()
            .find(|s| s.virtual_address <= rva && rva < s.virtual_address + s.virtual_size.max(s.raw_size))
            .map(|s| (s.raw_pointer + rva - s.virtual_address) as usize)
            .ok_or_else(|| format!("rva {rva:#x} is in no section"))
    };

    let raw_pointer = u32::try_from(image.len()).map_err(|_| "image too large")?;
    if raw_pointer % file_alignment != 0 {
        return Err(format!(
            "file size {raw_pointer:#x} is not a multiple of the file alignment {file_alignment:#x}"
        ));
    }
    let virtual_address = align_up(size_of_image, section_alignment);

    // The import directory: every descriptor as it is, then ours, then the terminator, then
    // the DLL name. Ours borrows the first descriptor's thunk list terminator as its own
    // (empty) list and its IAT slot, exactly as the launcher does.
    let import_offset = rva_to_offset(import_rva)?;
    let mut count = 0;
    while image
        .get(import_offset + count * DESCRIPTOR_SIZE..import_offset + (count + 1) * DESCRIPTOR_SIZE)
        .ok_or("import directory runs off the file")?
        .iter()
        .any(|&b| b != 0)
    {
        count += 1;
    }
    if count == 0 {
        return Err("the import directory is empty".into());
    }
    let first_thunks = u32_at(image, import_offset)?;
    let first_iat = u32_at(image, import_offset + 16)?;
    let borrowed_terminator = first_thunks + 4;
    if u32_at(image, rva_to_offset(borrowed_terminator)?)? != 0 {
        return Err("the first import descriptor does not have a single thunk; the recipe does not fit this image".into());
    }

    let mut blob = Vec::with_capacity(SECTION_RAW_SIZE as usize);
    blob.extend_from_slice(&image[import_offset..import_offset + count * DESCRIPTOR_SIZE]);
    let name_rva = virtual_address + ((count + 2) * DESCRIPTOR_SIZE) as u32;
    blob.extend_from_slice(&borrowed_terminator.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes());
    blob.extend_from_slice(&name_rva.to_le_bytes());
    blob.extend_from_slice(&first_iat.to_le_bytes());
    blob.extend_from_slice(&[0; DESCRIPTOR_SIZE]);
    blob.extend_from_slice(DLL_NAME);
    let import_size = blob.len() as u32;
    blob.resize(SECTION_RAW_SIZE as usize, 0);

    let mut out = image.to_vec();
    put_u16(&mut out, pe + 6, (section_count + 1) as u16);
    put_u32(&mut out, opt + 0x38, virtual_address + SECTION_VIRTUAL_SIZE);
    put_u32(&mut out, import_dir, virtual_address);
    put_u32(&mut out, import_dir + 4, import_size);
    for i in 0..=section_count {
        let h = table + i * SECTION_HEADER_SIZE;
        if i == section_count {
            out[h..h + 8].copy_from_slice(&SECTION_NAME);
            put_u32(&mut out, h + 8, SECTION_VIRTUAL_SIZE);
            put_u32(&mut out, h + 12, virtual_address);
            put_u32(&mut out, h + 16, SECTION_RAW_SIZE);
            put_u32(&mut out, h + 20, raw_pointer);
            put_u32(&mut out, h + 36, SECTION_CHARACTERISTICS);
        }
        put_u32(&mut out, h + 24, STAMP_RELOCATIONS);
        put_u32(&mut out, h + 28, STAMP_LINENUMBERS);
        put_u16(&mut out, h + 32, STAMP_RELOCATION_COUNT);
        put_u16(&mut out, h + 34, STAMP_LINENUMBER_COUNT);
    }
    out.extend_from_slice(&blob);
    Ok(out)
}

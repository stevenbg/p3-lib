use std::{fs, path::PathBuf, process::exit};

use clap::{Parser, Subcommand};
use log::LevelFilter;
use p3_rou::{decompress::decompress_file, TradeRouteFile, TradeRouteStop};
use serde::Deserialize;

const STOP_SIZE: usize = 220;
/// The game's sentinel for "as much as possible", stored unscaled.
const MAX_AMOUNT: i32 = 1_000_000_000;
/// Set on the first stop of every route the game saves.
const FIRST_STOP_MARKER: u8 = 0x04;
const FLAG_R: u8 = 0x01;
const FLAG_X: u8 = 0x00;
const FLAG_NONE: u8 = 0x09;

/// The ware display order the game writes into every saved route.
const DEFAULT_ORDER: [u8; 24] = [
    0x03, 0x13, 0x08, 0x02, 0x00, 0x11, 0x05, 0x0c, 0x0d, 0x01, 0x10, 0x0f, 0x12, 0x04, 0x09, 0x06, 0x0b, 0x0a, 0x07, 0x0e, 0x15, 0x17, 0x16, 0x14,
];

/// Ware names and their raw-unit scaling, indexed by ware id.
const WARES: [(&str, i32); 24] = [
    ("Grain", 2000),
    ("Meat", 2000),
    ("Fish", 2000),
    ("Beer", 200),
    ("Salt", 200),
    ("Honey", 200),
    ("Spices", 200),
    ("Wine", 200),
    ("Cloth", 200),
    ("Skins", 200),
    ("WhaleOil", 200),
    ("Timber", 2000),
    ("IronGoods", 200),
    ("Leather", 200),
    ("Wool", 2000),
    ("Pitch", 200),
    ("PigIron", 2000),
    ("Hemp", 2000),
    ("Pottery", 200),
    ("Bricks", 2000),
    ("Sword", 10),
    ("Bow", 10),
    ("Crossbow", 10),
    ("Carbine", 10),
];

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Decode a .rou file (compressed or uncompressed) and print its stops
    Dump {
        /// Path to the .rou file
        file: PathBuf,
    },
    /// Generate an uncompressed .rou file from a TOML route description
    Generate {
        /// Path to the route description TOML
        input: PathBuf,

        /// Path of the .rou file to write
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct RouteConfig {
    stops: Vec<StopConfig>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct StopConfig {
    /// Town index within the savegame's town list.
    town: u8,
    /// Stop flag as shown in the route window: "R" (repair), "X", or "-". Defaults to "X".
    flag: Option<String>,
    /// Transfers from the trading office onto the ship.
    #[serde(default)]
    load: Vec<TransferConfig>,
    /// Transfers from the ship into the trading office.
    #[serde(default)]
    unload: Vec<TransferConfig>,
    /// Sales to the town at a minimum price.
    #[serde(default)]
    sell: Vec<SellConfig>,
    /// Purchases from the town at a maximum price.
    #[serde(default)]
    buy: Vec<BuyConfig>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct TransferConfig {
    ware: String,
    /// Amount in in-game units (Last/barrels/pieces). Omit for "as much as possible".
    amount: Option<i32>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SellConfig {
    ware: String,
    /// Amount in in-game units (Last/barrels/pieces). Omit for "as much as possible".
    amount: Option<i32>,
    min_price: i32,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct BuyConfig {
    ware: String,
    /// Amount in in-game units (Last/barrels/pieces). Omit for "as much as possible".
    amount: Option<i32>,
    max_price: i32,
}

pub fn main() {
    simple_logger::SimpleLogger::new().with_level(LevelFilter::Info).env().init().unwrap();
    let args = Args::parse();

    match args.command {
        Command::Dump { file } => dump(&file),
        Command::Generate { input, output } => generate(&input, &output),
    }
}

fn dump(file: &PathBuf) {
    let data = decompress_file(file);
    if data.len() % STOP_SIZE != 0 {
        eprintln!("{} decompressed bytes are not a multiple of the stop size {STOP_SIZE}", data.len());
        exit(1);
    }
    println!("{}: {} stops", file.display(), data.len() / STOP_SIZE);

    for (n, stop) in data.chunks_exact(STOP_SIZE).enumerate() {
        let town = stop[2];
        let action = stop[3];
        let marker = if action & FIRST_STOP_MARKER != 0 { ", first-stop marker" } else { "" };
        let flag = match action & !FIRST_STOP_MARKER {
            FLAG_R => "R",
            FLAG_X => "X",
            FLAG_NONE => "-",
            _ => "?",
        };
        println!("\nstop {n}: town {town:#04x}, flag {flag} (action {action:#04x}{marker})");
        println!("  {:<10} {:>10} {:>12} {:>10}  operation", "ware", "price", "amount_raw", "amount");
        for (i, &(name, scaling)) in WARES.iter().enumerate() {
            let price = i32::from_le_bytes(stop[28 + i * 4..][..4].try_into().unwrap());
            let amount = i32::from_le_bytes(stop[124 + i * 4..][..4].try_into().unwrap());
            if amount == 0 {
                continue;
            }
            let operation = match (price, amount) {
                (0, a) if a > 0 => "load office -> ship".to_string(),
                (0, _) => "unload ship -> office".to_string(),
                (p, a) if p > 0 && a > 0 => format!("sell to town, min price {p}"),
                (p, a) if p < 0 && a > 0 => format!("buy from town, max price {}", -p),
                _ => "unknown".to_string(),
            };
            let display_amount = if amount == MAX_AMOUNT {
                "max".to_string()
            } else {
                format!("{}", amount / scaling)
            };
            println!("  {name:<10} {price:>10} {amount:>12} {display_amount:>10}  {operation}");
        }
    }
}

fn generate(input: &PathBuf, output: &PathBuf) {
    let config = fs::read_to_string(input).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {e}", input.display());
        exit(1);
    });
    let config: RouteConfig = toml::from_str(&config).unwrap_or_else(|e| {
        eprintln!("Failed to parse {}: {e}", input.display());
        exit(1);
    });
    if config.stops.is_empty() {
        eprintln!("The route needs at least one stop");
        exit(1);
    }

    let mut stops = Vec::with_capacity(config.stops.len());
    for (n, stop) in config.stops.iter().enumerate() {
        let mut action = match stop.flag.as_deref() {
            None | Some("X") | Some("x") => FLAG_X,
            Some("R") | Some("r") => FLAG_R,
            Some("-") => FLAG_NONE,
            Some(other) => {
                eprintln!("Stop {n}: unknown flag {other:?}, expected \"R\", \"X\" or \"-\"");
                exit(1);
            }
        };
        if n == 0 {
            action |= FIRST_STOP_MARKER;
        }

        let mut price = [0i32; 24];
        let mut amount = [0i32; 24];
        let mut occupied = [false; 24];
        let mut set_slot = |ware: &str, slot_price: i32, slot_amount: i32, occupied: &mut [bool; 24]| {
            let index = ware_index(ware).unwrap_or_else(|| {
                eprintln!("Stop {n}: unknown ware {ware:?}");
                exit(1);
            });
            if occupied[index] {
                eprintln!("Stop {n}: ware {ware:?} is configured more than once");
                exit(1);
            }
            occupied[index] = true;
            price[index] = slot_price;
            amount[index] = slot_amount;
        };

        for transfer in &stop.load {
            set_slot(&transfer.ware, 0, scaled_amount(&transfer.ware, transfer.amount, n), &mut occupied);
        }
        for transfer in &stop.unload {
            set_slot(&transfer.ware, 0, -scaled_amount(&transfer.ware, transfer.amount, n), &mut occupied);
        }
        for sale in &stop.sell {
            if sale.min_price <= 0 {
                eprintln!("Stop {n}: ware {:?} needs a positive min_price", sale.ware);
                exit(1);
            }
            set_slot(&sale.ware, sale.min_price, scaled_amount(&sale.ware, sale.amount, n), &mut occupied);
        }
        for purchase in &stop.buy {
            if purchase.max_price <= 0 {
                eprintln!("Stop {n}: ware {:?} needs a positive max_price", purchase.ware);
                exit(1);
            }
            set_slot(&purchase.ware, -purchase.max_price, scaled_amount(&purchase.ware, purchase.amount, n), &mut occupied);
        }

        stops.push(TradeRouteStop {
            town_index: stop.town,
            action,
            order: DEFAULT_ORDER,
            price,
            amount,
        });
    }

    let data = TradeRouteFile { stops }.serialize();
    fs::write(output, &data).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {e}", output.display());
        exit(1);
    });
    println!("Wrote {} stops ({} bytes) to {}", config.stops.len(), data.len(), output.display());
}

fn scaled_amount(ware: &str, amount: Option<i32>, stop: usize) -> i32 {
    let Some(amount) = amount else {
        return MAX_AMOUNT;
    };
    let Some(index) = ware_index(ware) else {
        eprintln!("Stop {stop}: unknown ware {ware:?}");
        exit(1);
    };
    if amount <= 0 {
        eprintln!("Stop {stop}: ware {ware:?} needs a positive amount");
        exit(1);
    }
    match amount.checked_mul(WARES[index].1) {
        Some(scaled) => scaled,
        None => {
            eprintln!("Stop {stop}: amount {amount} of {ware:?} overflows");
            exit(1);
        }
    }
}

fn ware_index(name: &str) -> Option<usize> {
    let normalized: String = name.chars().filter(|c| !c.is_whitespace() && *c != '_').collect::<String>().to_lowercase();
    WARES.iter().position(|(ware, _)| ware.to_lowercase() == normalized)
}

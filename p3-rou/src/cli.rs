use std::{collections::BTreeMap, fs, path::PathBuf, process::exit};

use clap::{Parser, Subcommand};
use log::LevelFilter;
use p3_rou::{decompress::decompress_file, TradeRouteFile, TradeRouteStop};
use serde::Deserialize;

const STOP_SIZE: usize = 220;
/// The game's sentinel for "as much as possible", stored unscaled.
const MAX_AMOUNT: i32 = 1_000_000_000;
/// Raw units per in-game unit: wares measured in loads (1 load = 10 barrels).
const LOAD_SCALING: i32 = 2000;
/// Raw units per in-game unit: wares measured in barrels.
const BARREL_SCALING: i32 = 200;
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
    /// Write a route of the given type from the reference goods table
    Write {
        /// Route layout
        #[arg(short = 't', long = "type", value_enum)]
        route_type: RouteType,

        /// Number of citizens to supply; unused by the suck type
        #[arg(short, long, default_value_t = 1000)]
        citizens: u32,

        /// Path of the .rou file to write
        #[arg(short, long)]
        output: PathBuf,

        /// Reference goods table TOML; defaults to the built-in table
        #[arg(short, long)]
        reference: Option<PathBuf>,

        /// Town index of the load stop; must be a town with a trading office, or the game blanks the load orders. Savegame-specific, see `dump`
        #[arg(long, default_value_t = 0, value_parser = parse_town_index)]
        load_town: u8,

        /// Town index of the sell stop. Savegame-specific, see `dump`
        #[arg(long, default_value_t = 0, value_parser = parse_town_index)]
        sell_town: u8,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum RouteType {
    /// load, sell max, take everything, put back calculated quantities, return
    #[value(name = "5stop")]
    FiveStop,
    /// like 5stop, but swaps the office stock one unit category at a time to bound the needed ship space
    #[value(name = "6stop")]
    SixStop,
    /// park in the load town: unload everything (with repair), then keep buying all goods at the reference buy prices
    #[value(name = "suck")]
    Suck,
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

/// Reference data for all trade wares; `citizens` is the scaling baseline for supply quantities.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SupplyReference {
    citizens: u32,
    goods: BTreeMap<String, ReferenceGood>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct ReferenceGood {
    /// Amount in in-game units per `citizens` inhabitants; absent for goods that are not part of citizens' supply.
    supply: Option<i32>,
    /// Minimum sell price used by supply routes.
    sell_price: i32,
    /// Maximum buy price, used by the suck route type.
    buy_price: i32,
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
        Command::Write {
            route_type,
            citizens,
            output,
            reference,
            load_town,
            sell_town,
        } => write_route(route_type, citizens, &output, reference.as_ref(), load_town, sell_town),
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
        let order = &stop[4..28];
        if order != DEFAULT_ORDER {
            let names: Vec<&str> = order.iter().filter_map(|&w| WARES.get(w as usize).map(|(name, _)| *name)).collect();
            println!("  instruction order: {}", names.join(", "));
        }
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
            let display_amount = match amount {
                MAX_AMOUNT => "max".to_string(),
                a if a == -MAX_AMOUNT => "-max".to_string(),
                a => format!("{}", a / scaling),
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
            set_slot(
                &purchase.ware,
                -purchase.max_price,
                scaled_amount(&purchase.ware, purchase.amount, n),
                &mut occupied,
            );
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

fn write_route(route_type: RouteType, citizens: u32, output: &PathBuf, reference: Option<&PathBuf>, load_town: u8, sell_town: u8) {
    let reference_str = match reference {
        Some(path) => fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Failed to read {}: {e}", path.display());
            exit(1);
        }),
        None => include_str!("supply_reference.toml").to_string(),
    };
    let reference: SupplyReference = toml::from_str(&reference_str).unwrap_or_else(|e| {
        eprintln!("Failed to parse the reference goods table: {e}");
        exit(1);
    });
    if reference.goods.is_empty() {
        eprintln!("The reference goods table is empty");
        exit(1);
    }
    if reference.citizens == 0 {
        eprintln!("The reference citizens count must be positive");
        exit(1);
    }

    let announce_supply = !matches!(route_type, RouteType::Suck);
    let mut load_amount = [0i32; 24];
    let mut sell_prices = [0i32; 24];
    let mut buy_price = [0i32; 24];
    if announce_supply {
        println!("Supplying {citizens} citizens (reference: {}):", reference.citizens);
    }
    for (ware, good) in &reference.goods {
        let Some(index) = ware_index(ware) else {
            eprintln!("Unknown ware {ware:?} in the reference goods table");
            exit(1);
        };
        if good.sell_price <= 0 || good.buy_price <= 0 {
            eprintln!("Ware {ware:?} needs positive prices in the reference goods table");
            exit(1);
        }
        buy_price[index] = good.buy_price;
        // Goods without a supply amount are not part of citizens' supply.
        let Some(supply) = good.supply else {
            continue;
        };
        if supply <= 0 {
            eprintln!("Ware {ware:?} needs a positive supply amount in the reference goods table");
            exit(1);
        }
        // Scale linearly, rounding up so the supply never undershoots.
        let units = (supply as i64 * citizens as i64 + reference.citizens as i64 - 1) / reference.citizens as i64;
        let raw = units * WARES[index].1 as i64;
        let Ok(raw) = i32::try_from(raw) else {
            eprintln!("Scaled amount of {ware:?} overflows");
            exit(1);
        };
        load_amount[index] = raw;
        sell_prices[index] = good.sell_price;
        if announce_supply {
            println!("  {:<10} {units:>6} @ min {}", WARES[index].0, good.sell_price);
        }
    }

    // Sell the reference goods without an amount limit, at the reference prices.
    let mut sell_max = [0i32; 24];
    // Take back exactly the calculated quantities of the reference goods.
    let mut unload_calculated = [0i32; 24];
    for (i, load) in load_amount.iter().enumerate() {
        if *load != 0 {
            sell_max[i] = MAX_AMOUNT;
            unload_calculated[i] = -load;
        }
    }

    let stop = |town_index: u8, action: u8, price: [i32; 24], amount: [i32; 24]| TradeRouteStop {
        town_index,
        action,
        order: DEFAULT_ORDER,
        price,
        amount,
    };
    // The order array is the stop's instruction order; put the unloading wares first so
    // the ship frees up space before taking new cargo on.
    let partitioned_order = |unloads_first: [i32; 24]| {
        let mut order = [0u8; 24];
        let mut n = 0;
        for &ware in DEFAULT_ORDER.iter().filter(|&&w| unloads_first[w as usize] < 0) {
            order[n] = ware;
            n += 1;
        }
        for &ware in DEFAULT_ORDER.iter().filter(|&&w| unloads_first[w as usize] >= 0) {
            order[n] = ware;
            n += 1;
        }
        order
    };

    // Every type starts by loading the calculated quantities at the source (repairing there)
    // and ends by unloading the whole ship back into the source office. The stops in between
    // reset the target office stock to exactly the calculated quantities and take the surplus
    // along; the 6stop variant phases this by unit category to bound the required ship space.
    let stops = match route_type {
        RouteType::FiveStop => vec![
            stop(load_town, FLAG_R | FIRST_STOP_MARKER, [0i32; 24], load_amount),
            stop(sell_town, FLAG_X, sell_prices, sell_max),
            stop(sell_town, FLAG_X, [0i32; 24], [MAX_AMOUNT; 24]),
            stop(sell_town, FLAG_X, [0i32; 24], unload_calculated),
            stop(load_town, FLAG_X, [0i32; 24], [-MAX_AMOUNT; 24]),
        ],
        RouteType::SixStop => {
            let mut unload_loads_take_barrels = [0i32; 24];
            let mut unload_barrels_take_loads = [0i32; 24];
            let mut unload_loads_calculated = [0i32; 24];
            for (i, &(_, scaling)) in WARES.iter().enumerate() {
                match scaling {
                    LOAD_SCALING => {
                        unload_loads_take_barrels[i] = -MAX_AMOUNT;
                        unload_barrels_take_loads[i] = MAX_AMOUNT;
                        unload_loads_calculated[i] = unload_calculated[i];
                    }
                    BARREL_SCALING => {
                        unload_loads_take_barrels[i] = MAX_AMOUNT;
                        unload_barrels_take_loads[i] = unload_calculated[i];
                    }
                    _ => {} // weapons take part only in the initial load, the sale, and the final unload
                }
            }
            let mut swap_barrels_stop = stop(sell_town, FLAG_X, [0i32; 24], unload_loads_take_barrels);
            swap_barrels_stop.order = partitioned_order(unload_loads_take_barrels);
            let mut swap_loads_stop = stop(sell_town, FLAG_X, [0i32; 24], unload_barrels_take_loads);
            swap_loads_stop.order = partitioned_order(unload_barrels_take_loads);
            vec![
                stop(load_town, FLAG_R | FIRST_STOP_MARKER, [0i32; 24], load_amount),
                stop(sell_town, FLAG_X, sell_prices, sell_max),
                swap_barrels_stop,
                swap_loads_stop,
                stop(sell_town, FLAG_X, [0i32; 24], unload_loads_calculated),
                stop(load_town, FLAG_X, [0i32; 24], [-MAX_AMOUNT; 24]),
            ]
        }
        RouteType::Suck => {
            let mut buy_prices = [0i32; 24];
            let mut buy_max = [0i32; 24];
            for (i, &price) in buy_price.iter().enumerate() {
                if price > 0 {
                    buy_prices[i] = -price;
                    buy_max[i] = MAX_AMOUNT;
                }
            }
            let mut stops = vec![stop(load_town, FLAG_R | FIRST_STOP_MARKER, [0i32; 24], [-MAX_AMOUNT; 24])];
            for _ in 0..5 {
                stops.push(stop(load_town, FLAG_X, buy_prices, buy_max));
            }
            stops
        }
    };

    let stop_count = stops.len();
    let data = TradeRouteFile { stops }.serialize();
    fs::write(output, &data).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {e}", output.display());
        exit(1);
    });
    let description = match route_type {
        RouteType::FiveStop | RouteType::SixStop => {
            format!("load in town {load_town:#04x}, sell and restock in town {sell_town:#04x}, unload everything back in town {load_town:#04x}")
        }
        RouteType::Suck => format!("unload everything (with repair) and buy all goods at the reference buy prices, all in town {load_town:#04x}"),
    };
    println!("Wrote {stop_count} stops ({} bytes) to {}: {description}", data.len(), output.display());
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

/// Parses a town index given as decimal or as hex with a 0x prefix, the way `dump` prints it.
fn parse_town_index(input: &str) -> Result<u8, String> {
    let result = match input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16),
        None => input.parse(),
    };
    result.map_err(|e| e.to_string())
}

/// Looks up a ware by its exact `WareId` identifier (e.g. "PigIron").
fn ware_index(name: &str) -> Option<usize> {
    WARES.iter().position(|(ware, _)| *ware == name)
}

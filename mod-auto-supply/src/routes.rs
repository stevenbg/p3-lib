//! The route templates and everything they stand on: F1/F2 supply, F3 collect, F4
//! trade, CTRL+F3 fetch, DEL clear - plus the route pool layout, the ship naming and
//! the route file loader. Key registration and dispatch live in [crate::ffi].

use std::mem;
use std::sync::atomic::{AtomicU32, Ordering};

use log::{debug, error, info, warn};
use num_traits::FromPrimitive;
use p3_api::{
    data::enums::WareId,
    game_world::GAME_WORLD_PTR,
    operation::Operation,
    operations::OPERATIONS_PTR,
    town::get_town_name,
    ui::ui_ship_panel::UIShipPanelPtr,
};
use p3_rou::{builder, builder::MAX_DISPLAYABLE_STOPS, TradeRouteStop};

use crate::prices::{buy_price, sell_price, PriceLevel};

// The price levels the generated orders use. Every automatic price in the mod comes from
// one of these five, so retuning is a one-line change per feature:
//
// | feature | buy | sell |
// |-|-|-|
// | F1 office setup | Q (par, 1.00) | Q (supply price, 1.40) |
// | F1 goods-dialog fill | [STOP_BUY_LEVEL] R | [STOP_SELL_LEVEL] R |
// | F4 trade template | [STOP_BUY_LEVEL] R | [STOP_SELL_LEVEL] R |
// | F1/F2 supply templates | [COLLECT_BUY_LEVEL] E | [SUPPLY_SELL_LEVEL] Q |
// | F3 collection template | [COLLECT_BUY_LEVEL] E | - |
//
// The CTRL+F3 fetch template is deliberately absent: it copies the buy prices off the
// ship's own first stop, so its prices are the player's, not a level of ours. Which is
// also how it gets a per-ware price instead of one letter for the whole list.
//
// The office setup's Q is written at its own call site because it prices existing orders
// rather than building a stop.

/// What a trade stop pays for the wares its town produces - the F4 template and the
/// goods-dialog fill, which build the same shape of stop.
pub(crate) const STOP_BUY_LEVEL: PriceLevel = PriceLevel::R;
/// What a trade stop asks for everything its town does not produce.
pub(crate) const STOP_SELL_LEVEL: PriceLevel = PriceLevel::R;
/// What the collection route and the office-less targets of a supply route pay. Shared
/// deliberately: both are "haul home what this town has that we do not produce".
const COLLECT_BUY_LEVEL: PriceLevel = PriceLevel::E;
/// What a supply route asks for the goods it delivers to a target.
const SUPPLY_SELL_LEVEL: PriceLevel = PriceLevel::Q;
/// Wares the trade template (F4) and the goods-dialog fill leave in the town even where
/// it produces them: their margin does not justify the cargo space early on (grain, hemp
/// and timber are bulky loads goods). ALT buys them too. Also the wares the collection
/// route never buys - there unconditionally.
pub(crate) const NO_BUY_WARES: [WareId; 6] = [WareId::Pitch, WareId::Timber, WareId::Salt, WareId::Bricks, WareId::Grain, WareId::Hemp];
/// Low-value industry inputs, worth skipping when hold space is tight: what a target
/// consumes of them is mostly business demand, and they crowd out goods with a better
/// margin. **Excluded only when ALT is held** with a route key - a plain route supplies
/// everything the target consumes. Whatever accumulates in the target office is hauled
/// home either way. (Timber is not on this list - as the worst of the loads goods it is
/// simply ordered last, see p3-rou's cargo_order.)
const NO_SUPPLY_WARES: [WareId; 4] = [WareId::Bricks, WareId::PigIron, WareId::Pitch, WareId::Hemp];

/// The game's route file loader: thiscall(this, base_name) -> decompressed buffer. It
/// forms the path "save\AutoRoute\<name>.rou" itself, so we write the file there and
/// pass the base name.
const ROUTE_LOADER_ADDRESS: u32 = 0x004d5ee0;
const ROUTE_LOADER_THIS: u32 = 0x006dd728;
/// operations struct (0x6df2f0): +0x930 = route buffer pointer, +0x934 = target ship.
const OPERATIONS_ROUTE_BUFFER: *mut u32 = 0x006dfc20 as _;
const OPERATIONS_ROUTE_SHIP: *mut u32 = 0x006dfc24 as _;
const ROUTE_BASE_NAME: &str = "_autosupply";
const ROUTE_FILE_PATH: &str = "save/AutoRoute/_autosupply.rou";
/// The ship's route as it was before a key changed it, so it can be loaded back through
/// the route window's File button.
const ROUTE_BACKUP_PATH: &str = "save/AutoRoute/_backup.rou";
const FIRST_STOP_MARKER: u8 = 0x04;

/// The loader takes a pointer to an MFC-style string object: a single pointer to char
/// data, with refcount/alloc/length in the 0xc bytes before it. [0x6c7cd0] holds the
/// shared empty-string header, which is what a default-constructed object points past.
const STRING_EMPTY_HEADER_PTR: *const u32 = 0x006c7cd0 as _;
/// thiscall(this, *const u8) -> this: construct/assign from a C string.
const STRING_CTOR_FROM_CSTR: u32 = 0x0064f390;
/// thiscall(this): release the string data.
const STRING_DTOR: u32 = 0x0064f253;

/// The ship currently shown in the ship panel (the map selection).
unsafe fn selected_ship_index() -> Option<u16> {
    UIShipPanelPtr::new().get_selected_ship_index()
}

/// The ship a route key should act on for a given selection: the selected ship itself, or -
/// when it sails in a convoy - the convoy's LEAD ship.
///
/// Every route key needs this, because a convoy keeps its whole route on the lead ship
/// (`ship+0x132`; measured, the other members read `0xFFFF`) while the ship panel reports a
/// member. Selecting the convoy *as a whole* reports a member too, so without this a route
/// key on a convoy reads an empty route and refuses. Acting on the leader is also what the
/// game itself does: it attaches a route loaded there to the whole convoy, and it names the
/// convoy after its leader, so the generated `-LueRosSte` name lands where it belongs.
///
/// Conservative by construction - it falls back to the selection whenever anything fails to
/// line up. `ship+0x8` is `0xFFFF` for a ship sailing alone (the game writes that into
/// `+0x6`/`+0x8` at `0x004E13F2`), which `get_convoy` rejects on its bounds check; and the
/// leader is only accepted if it names this same convoy back, so a member that has just left
/// one cannot redirect a key onto a ship the player did not select.
unsafe fn route_ship_index(selected: u16) -> u16 {
    let ships = p3_api::ships::ShipsPtr::new();
    let Some(ship) = ships.get_ship(selected) else { return selected };
    let convoy_index = ship.get_convoy_id();
    let Some(convoy) = ships.get_convoy(convoy_index) else { return selected };
    let lead = convoy.get_lead_ship_index();
    if lead == selected {
        return selected;
    }
    let Some(lead_ship) = ships.get_ship(lead) else {
        error!("convoy {convoy_index} names lead ship {lead}, which is out of range - acting on the selected ship {selected}");
        return selected;
    };
    if lead_ship.get_convoy_id() != convoy_index {
        error!(
            "convoy {convoy_index} names lead ship {lead}, but that ship is in convoy {} - acting on the selected ship {selected}",
            lead_ship.get_convoy_id()
        );
        return selected;
    }
    info!(
        "selection is ship {selected} {:?} of convoy {convoy_index}; acting on its lead ship {lead} {:?}, which carries the route",
        ship.get_name(),
        lead_ship.get_name()
    );
    lead
}

/// True if `ship_index` belongs to the player (walk the player merchant's ship chain).
unsafe fn is_player_ship(ship_index: u16) -> bool {
    let ships = p3_api::ships::ShipsPtr::new();
    let ships_size = ships.get_ships_size();
    let merchant = GAME_WORLD_PTR.get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16);
    let mut idx = merchant.get_first_ship_index();
    for _ in 0..2000 {
        if idx >= ships_size {
            break;
        }
        if idx == ship_index {
            return true;
        }
        let Some(ship) = ships.get_ship(idx) else { break };
        idx = ship.get_next_ship_index_of_merchant();
    }
    false
}

/// Read a 220-byte pool record into a TradeRouteStop (the file layout, head field dropped).
unsafe fn read_pool_stop(record: u32) -> TradeRouteStop {
    let mut order = [0u8; 24];
    core::ptr::copy_nonoverlapping((record + 4) as *const u8, order.as_mut_ptr(), 24);
    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    for i in 0..24u32 {
        price[i as usize] = *((record + 28 + i * 4) as *const i32);
        amount[i as usize] = *((record + 124 + i * 4) as *const i32);
    }
    TradeRouteStop {
        town_index: *((record + 2) as *const u8),
        action: *((record + 3) as *const u8),
        order,
        price,
        amount,
    }
}

/// Read a ship's current route by walking the pool chain, rotated so the logical first
/// stop (the one carrying the 0x04 marker) is first. Empty if the ship has no route.
/// The pool record addresses of a ship's route, rotated so the logical first stop (the
/// one carrying the 0x04 marker) is first - the order the route window displays.
unsafe fn route_stop_records(ship_index: u16) -> Vec<u32> {
    let ships = p3_api::ships::ShipsPtr::new();
    let Some(ship) = ships.get_ship(ship_index) else {
        return Vec::new();
    };
    let pool = *ROUTE_STOP_POOL;
    let pool_count = *ROUTE_STOP_POOL_COUNT;
    let head = *((ship.address + SHIP_ROUTE_HEAD_OFFSET) as *const u16);
    if pool == 0 || head >= pool_count {
        return Vec::new();
    }
    let mut records = Vec::new();
    let mut idx = head;
    for _ in 0..64 {
        let record = pool + idx as u32 * ROUTE_STOP_SIZE;
        records.push(record);
        let next = *(record as *const u16);
        if next == idx || next >= pool_count {
            break;
        }
        idx = next;
        if idx == head {
            break;
        }
    }
    if let Some(pos) = records.iter().position(|&r| *((r + 3) as *const u8) & FIRST_STOP_MARKER != 0) {
        records.rotate_left(pos);
    }
    records
}

pub(crate) unsafe fn read_ship_route(ship_index: u16) -> Vec<TradeRouteStop> {
    route_stop_records(ship_index).into_iter().map(|record| read_pool_stop(record)).collect()
}

/// Save the ship's current route to [ROUTE_BACKUP_PATH], loadable from the route
/// window's File button as "_backup". Returns the stops for reuse.
unsafe fn backup_route(ship_index: u16) -> Vec<TradeRouteStop> {
    let previous = read_ship_route(ship_index);
    if !previous.is_empty() {
        let count = previous.len();
        let backup = p3_rou::TradeRouteFile { stops: previous.clone() }.serialize();
        match std::fs::write(ROUTE_BACKUP_PATH, &backup) {
            Ok(()) => info!("route: saved the previous {count} stops to {ROUTE_BACKUP_PATH}"),
            Err(e) => error!("route: failed to back up the previous route: {e}"),
        }
    }
    previous
}

/// DEL: clear the selected ship's route by enqueueing the game's own stop-removal
/// operation for every stop - the op the route panel's town "none" selection sends,
/// identifying each stop by its POOL INDEX (stable across the removals, unlike
/// positions). Guarded on the goods dialog being closed: it displays a stop of this
/// route, and the removals free the pool records it points into.
pub(crate) unsafe fn on_clear_route_hotkey() {
    if crate::goods_dialog::goods_dialog_stop().is_some() {
        crate::ffi::notify("Clear route: close the goods dialog first");
        return;
    }
    let Some(selected) = selected_ship_index() else {
        crate::ffi::notify("Clear route: no ship selected");
        return;
    };
    // A convoy's route lives on its lead ship, so clear it there - see [route_ship_index].
    let ship_index = route_ship_index(selected);
    if !is_player_ship(ship_index) {
        crate::ffi::notify("Clear route: the selected ship is not yours");
        return;
    }
    let previous = backup_route(ship_index);
    if previous.is_empty() {
        crate::ffi::notify("Clear route: the ship has no route");
        return;
    }
    // Deactivate first - the removals do not touch the active flag - then remove every
    // stop, the way transfer_loaded_traderoute also deactivates before rebuilding.
    OPERATIONS_PTR.enqueue_operation(Operation::SetTradeRouteActive {
        ship_index: ship_index as u32,
        active: false,
    });
    let pool = *ROUTE_STOP_POOL;
    for record in route_stop_records(ship_index) {
        OPERATIONS_PTR.enqueue_operation(Operation::RemoveTradeRouteStop {
            stop_pool_index: (record - pool) / ROUTE_STOP_SIZE,
            ship_index: ship_index as u32,
        });
    }
    let ships = p3_api::ships::ShipsPtr::new();
    let name = ships.get_ship(ship_index).map(|s| s.get_name()).unwrap_or_default();

    // The route keys rename the ship after its towns, so a cleared ship would keep a name
    // describing a route it no longer has. Put it back on a pool name, the way a
    // newly built ship gets one.
    let renamed = match random_ship_name() {
        Some(pool_name) => {
            rename_ship(ship_index, &pool_name);
            format!(", renamed {pool_name}")
        }
        None => String::new(),
    };
    crate::ffi::notify(&format!("Route cleared: {} stops removed from {name}{renamed}", previous.len()));
}

/// A name drawn from the game's own ship-name pool (`scripts/NamenSchiffe_eng.txt`), the
/// list a newly built ship is named from.
///
/// The game picks its index by stepping a shared counter (`state = (state + step) % 307`
/// at `0x0050E1A6`, rejecting values past the pool count). This deliberately does **not**
/// reuse that: the state lives on a game object and advancing it would perturb the
/// sequence the game's own naming draws from. A local mix of the game clock and a call
/// counter is enough - the only requirement is that pressing DEL twice does not hand out
/// the same name twice in the same tick.
unsafe fn random_ship_name() -> Option<String> {
    let count = p3_api::names::ship_name_count();
    if count == 0 {
        warn!("clear route: the ship-name pool is empty - leaving the name alone");
        return None;
    }
    let nth = NAME_PICKS.fetch_add(1, Ordering::Relaxed);
    let mut x = GAME_WORLD_PTR.get_game_time_raw() ^ nth.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    let name = p3_api::names::get_ship_name((x % count as u32) as u16)?;
    Some(name.iter().map(|&b| b as char).collect())
}

/// Counts calls to [random_ship_name] so two presses inside one game tick differ.
static NAME_PICKS: AtomicU32 = AtomicU32::new(0);

/// Apply a route file (written to ROUTE_FILE_PATH) to a ship, mimicking the game's own
/// route Load: build the base-name string object, set the target ship, call the loader,
/// hand the resulting buffer to transfer (which frees it).
unsafe fn apply_route_file(ship_index: u16) -> bool {
    let mut name: Vec<u8> = ROUTE_BASE_NAME.bytes().collect();
    name.push(0);

    // Start from the empty-string sentinel, like a default-constructed object, so the
    // constructor's "release the old data" path is a no-op instead of a wild free.
    let mut string_object: u32 = *STRING_EMPTY_HEADER_PTR + 0xc;
    let ctor: extern "thiscall" fn(*mut u32, *const u8) -> *mut u32 = mem::transmute(STRING_CTOR_FROM_CSTR);
    ctor(&mut string_object, name.as_ptr());
    debug!("add stop: name string built");

    *OPERATIONS_ROUTE_SHIP = ship_index as u32;
    let loader: extern "thiscall" fn(u32, *const u32) -> u32 = mem::transmute(ROUTE_LOADER_ADDRESS);
    let buffer = loader(ROUTE_LOADER_THIS, &string_object);
    debug!("add stop: loader returned buffer {buffer:#010x}");

    let dtor: extern "thiscall" fn(*mut u32) = mem::transmute(STRING_DTOR);
    dtor(&mut string_object);

    if buffer == 0 {
        return false;
    }
    *OPERATIONS_ROUTE_BUFFER = buffer;
    OPERATIONS_PTR.transfer_loaded_traderoute();
    true
}

/// True if the player has a trading office in the town. Route stops that transfer wares
/// to or from an office are wiped at load time in towns where there is none.
unsafe fn has_player_office(town_index: u8) -> bool {
    let merchant_index = OPERATIONS_PTR.get_player_merchant_index();
    GAME_WORLD_PTR.get_office_in_of(town_index as _, merchant_index as _).is_some()
}

/// Write the stops as the route file and apply them to the ship, normalising the
/// logical-first marker onto the first stop. Returns false and logs on failure.
///
/// Callers must have clamped to [`MAX_DISPLAYABLE_STOPS`] already - opening the auto-trade
/// window on a longer route crashes the game.
unsafe fn write_and_apply_route(ship_index: u16, mut stops: Vec<TradeRouteStop>) -> bool {
    for (i, stop) in stops.iter_mut().enumerate() {
        if i == 0 {
            stop.action |= FIRST_STOP_MARKER;
        } else {
            stop.action &= !FIRST_STOP_MARKER;
        }
    }
    let data = p3_rou::TradeRouteFile { stops }.serialize();
    if let Err(e) = std::fs::write(ROUTE_FILE_PATH, &data) {
        error!("route: failed to write {ROUTE_FILE_PATH}: {e}");
        return false;
    }
    if !apply_route_file(ship_index) {
        error!("route: loader failed (is fix_uncompressed_trade_route_loading.dll installed?)");
        return false;
    }
    true
}

/// The route templates the F1/F2/F3 keys can set, from `p3_rou::builder`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RouteKind {
    FiveStop,
    SixStop,
    Suck,
    /// One self-contained trade stop per target: buy what the town produces, sell what
    /// it does not, both at the extreme Y prices. Nothing is loaded at home and
    /// everything is unloaded there, so the route is a circuit that needs no
    /// consumption figures and no office at any target.
    Trade,
    /// A collection route for a hand-picked ware list: the wares are the buy orders the
    /// player left on the ship's own FIRST stop, and the targets are every town that
    /// produces at least one of them (with ALT, every town at all), visited in the
    /// shortest closed tour. Every stop buys the whole list at the prices that first stop
    /// carries, copied through as-is - the only template that does not price itself.
    ///
    /// The only template whose ware list is explicit rather than derived from
    /// production or consumption, and the only one that requires an existing route -
    /// the first stop is its input.
    Fetch,
}

/// F1/F2/F3: rebuild the selected ship's route from a supply template. The ship's current
/// route provides the towns: its FIRST stop's town becomes the home town, and the
/// remaining unique towns, in order, the targets (further occurrences of the home town
/// are ignored; a ship without a route uses the merchant's home town and the open town
/// view). The generated route repeats the template's action stops once per target,
/// bracketed by a home load stop (summed quantities) and a home unload stop - except the
/// collection templates ([RouteKind::Suck] and [RouteKind::Fetch]), which load nothing and
/// so use a **single** home stop that transfers the hold into the office
/// (`builder::collecting_route`); the route loops back to it, so a trailing home stop
/// would only repeat it. With shift held, the target is
/// just the currently open town and the generated stops are APPENDED to the existing
/// route instead of replacing it.
///
/// **What ALT means depends on the template**, because each has a different filter worth
/// relaxing - in both cases it is a ware filter, never a quantity:
///
/// | Template | plain | with ALT |
/// |-|-|-|
/// | F1 5stop, F2 6stop | supply everything the target consumes | leave out the [NO_SUPPLY_WARES] |
/// | F3 collect, F4 trade | leave the [NO_BUY_WARES] in the town | buy those too |
/// | CTRL+F3 fetch | call only at towns that produce a wanted ware | call at every town |
///
/// For the supply templates, quantities are a week of each target's citizen and business
/// consumption and prices the R levels. Wares a target produces itself are not supplied to
/// it; targets without a
/// player office get a combined sell-and-buy trade stop (buying their produce at T)
/// instead of the office-reset stops. F3 builds a collection route instead: one
/// buy stop per target at the R price, skipping the NO_BUY_WARES and everything the
/// home town produces itself, behind the single home transfer stop.
///
/// [RouteKind::Fetch] (CTRL+F3) is the exception to the paragraph above: it takes neither
/// its wares nor its targets from the same places. The ware list is the buy orders the
/// player left on the ship's FIRST stop, the targets are every town that produces one of
/// them, and their order is the shortest closed tour rather than the order they were
/// discovered in - so it needs an existing route and takes no SHIFT. Its ALT is the one
/// in the table above that filters TOWNS rather than wares.
pub(crate) unsafe fn on_route_hotkey(kind: RouteKind, append: bool, alt: bool, scale_to_lap: bool) {
    let Some(selected) = selected_ship_index() else {
        crate::ffi::notify("Route: no ship selected");
        return;
    };
    // A convoy carries its route, and its name, on the lead ship - see [route_ship_index].
    let ship_index = route_ship_index(selected);
    if !is_player_ship(ship_index) {
        crate::ffi::notify("Route: the selected ship is not yours");
        return;
    }
    let ships = p3_api::ships::ShipsPtr::new();
    let name = ships.get_ship(ship_index).map(|s| s.get_name()).unwrap_or_default();
    // Whether replacing or appending, the whole route gets rewritten, so keep the old
    // one loadable from the route window's File button ("_backup") in case this was a
    // mistake. The previous stops also provide the home and target towns.
    let previous = backup_route(ship_index);

    // The route's first stop is the home town; without a route, the merchant's home.
    let load_town = if !append && !previous.is_empty() {
        previous[0].town_index
    } else {
        GAME_WORLD_PTR
            .get_merchant(OPERATIONS_PTR.get_player_merchant_index() as u16)
            .get_hometown_index()
    };
    let load_town_name = get_town_name(load_town).unwrap_or_else(|| "<unknown>".into());

    // What a fetch route collects, and for how much: the buy orders the player left on the
    // ship's own first stop, wares and prices both. Resolved before the targets because it
    // decides them - and refused loudly, since this is the one input the template cannot
    // invent.
    let fetch_buys = if matches!(kind, RouteKind::Fetch) {
        let Some(first) = previous.first() else {
            crate::ffi::notify("Fetch route: the ship has no route - it needs a first stop carrying the buy orders to collect");
            return;
        };
        let prices = fetch_buy_prices(first);
        if !prices.iter().any(|&price| price > 0) {
            crate::ffi::notify(&format!(
                "Fetch route: the first stop ({load_town_name}) has no buy orders - mark the wares to collect there first"
            ));
            return;
        }
        Some(prices)
    } else {
        None
    };

    let targets: Vec<u8> = if let Some(buys) = &fetch_buys {
        // Every town that produces one of the wanted wares - or, with ALT, every town
        // there is - in the shortest closed tour from home. The ship's previous stops say
        // nothing here: the whole point is to discover the sources rather than list them
        // by hand.
        let towns = fetch_targets(load_town, buys, alt);
        if towns.is_empty() {
            crate::ffi::notify(&format!(
                "Fetch route: no town other than {load_town_name} produces [{}]",
                ware_names(buys).join(", ")
            ));
            return;
        }
        order_towns_by_distance(load_town, towns)
    } else if append {
        // Shift: append the template for the currently open town.
        let Some(town) = crate::ffi::current_town_index() else {
            crate::ffi::notify("Route: shift appends for the open town, but no town view is open");
            return;
        };
        vec![town]
    } else {
        let mut towns: Vec<u8> = Vec::new();
        for stop in previous.iter().skip(1) {
            if stop.town_index != load_town && !towns.contains(&stop.town_index) {
                towns.push(stop.town_index);
            }
        }
        // A ship without a route (or one only touching one town) falls back to the
        // open town view, preserving the old single-target workflow.
        if towns.is_empty() {
            match crate::ffi::current_town_index() {
                Some(town) if town != load_town => towns.push(town),
                _ => {
                    crate::ffi::notify("Route: no target towns in the current route and no town view open");
                    return;
                }
            }
        }
        towns
    };

    // The buy list, shared by the collection route and the office-less trade stops:
    // everything the home town does not produce itself, at the R price. (In trade stops,
    // sells take precedence per ware.)
    //
    // ALT widens it by including the NO_BUY_WARES - but only for the collection route,
    // whose whole job is buying. On the supply templates ALT already means the supply
    // filter, and letting it also widen the buying at office-less targets would have one
    // key freeing hold space and filling it again in the same press.
    let skip_no_buy = !(matches!(kind, RouteKind::Suck) && alt);
    let home_production = GAME_WORLD_PTR.get_town(load_town).get_production_values();
    let mut collect_buys = [0i32; 24];
    for ware_index in crate::ffi::TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        if home_production[i] > 0 || (skip_no_buy && NO_BUY_WARES.contains(&ware_id)) {
            continue;
        }
        collect_buys[i] = buy_price(ware_index, COLLECT_BUY_LEVEL);
    }


    // CTRL on the supply templates scales the load to the route's actual lap time
    // instead of the fixed week. The loop below runs once with 7 days; if the computed
    // lap differs, it runs a second time with the real figure - the stop TOWNS do not
    // depend on the quantities, so the first pass's shape is already the final one and
    // the duration computed from it is exact.
    let scale_to_lap = scale_to_lap && matches!(kind, RouteKind::FiveStop | RouteKind::SixStop);
    let mut days: i32 = 7;
    let (total_load, middle, described) = loop {
    let mut total_load = [0i32; 24];
    let mut middle: Vec<TradeRouteStop> = Vec::new();
    let mut described: Vec<String> = Vec::new();
    for &town_index in &targets {
        let town_name = get_town_name(town_index).unwrap_or_else(|| "<unknown>".into());
        if matches!(kind, RouteKind::Suck) {
            middle.push(builder::buy_stop(town_index, &collect_buys));
            described.push(town_name);
            continue;
        }
        if let Some(prices) = &fetch_buys {
            middle.push(builder::buy_stop(town_index, prices));
            described.push(town_name);
            continue;
        }
        if matches!(kind, RouteKind::Trade) {
            // ALT widens the buying here rather than narrowing the supplies: this
            // template supplies nothing to narrow.
            let (price, amount, bought, skipped, sold) = town_trade_basket(town_index, !alt);
            middle.push(builder::stop(town_index, builder::FLAG_X, price, amount));
            let left = if skipped.is_empty() {
                String::new()
            } else {
                format!(", selling not buying [{}]", skipped.join(", "))
            };
            described.push(format!("{town_name} (buying [{}]{left}, selling {sold})", bought.join(", ")));
            continue;
        }

        let (load_amount, sell_prices, supplied) = town_supply_basket(town_index, alt, days);
        for i in 0..24 {
            total_load[i] = total_load[i].saturating_add(load_amount[i]);
        }
        if has_player_office(town_index) {
            middle.extend(match kind {
                RouteKind::FiveStop => builder::five_stop_middle(town_index, &load_amount, &sell_prices),
                _ => builder::six_stop_middle(town_index, &load_amount, &sell_prices),
            });
            described.push(format!("{town_name} ({supplied} wares)"));
        } else {
            // No office: the reset stops would be wiped, so trade in one stop - sell
            // the supplies and buy the rest at T through the shared buy filter (no
            // NO_BUY_WARES, nothing the home town produces itself).
            middle.push(builder::trade_stop(town_index, &load_amount, &sell_prices, &collect_buys));
            described.push(format!("{town_name} ({supplied} wares, no office)"));
        }
    }

    if scale_to_lap && days == 7 {
        // The final stop sequence: the previous route when appending, then the
        // template's two-stop home bracket around the middle. Same-town legs are free,
        // so listing every stop also counts each one's 6-hour dwell.
        let mut towns: Vec<u8> = if append {
            previous.iter().map(|stop| stop.town_index).collect()
        } else {
            Vec::new()
        };
        towns.push(load_town);
        towns.extend(middle.iter().map(|stop| stop.town_index));
        towns.push(load_town);
        let ship_type = p3_api::ships::ShipsPtr::new().get_ship(ship_index).map(|ship| ship.get_type());
        match ship_type.and_then(|t| route_duration_days(&towns, t)) {
            Some(lap) if lap as i32 != days => {
                days = lap as i32;
                continue; // rebuild the baskets with the real lap
            }
            Some(_) => {}
            None => {
                warn!("route: lap duration unavailable (router failed a leg) - keeping the 7-day load");
                crate::ffi::notify("Route: lap time unavailable, loading a week");
            }
        }
    }
    break (total_load, middle, described);
    };

    // The two collection templates load nothing at home and only ever bring goods back,
    // so they use the one-stop home bracket: the leading home stop transfers the hold into
    // the office and the route loops straight back to it, making a trailing home stop
    // redundant. The supply and trade templates still need the load stop at the front.
    let template = if matches!(kind, RouteKind::Suck | RouteKind::Fetch) {
        builder::collecting_route(load_town, middle)
    } else {
        builder::bracketed_route(load_town, total_load, middle)
    };
    let stops = if append {
        let mut stops = previous;
        stops.extend(template);
        stops
    } else {
        template
    };
    // The auto-trade window has exactly 20 row widgets and no bound check, so a longer
    // route crashes the game the moment the player opens it - see MAX_DISPLAYABLE_STOPS.
    // Appending is what actually reaches this: a full "every town" fetch is well past 20.
    let mut stops = stops;
    if stops.len() > MAX_DISPLAYABLE_STOPS {
        let dropped = stops.len() - MAX_DISPLAYABLE_STOPS;
        stops.truncate(MAX_DISPLAYABLE_STOPS);
        warn!(
            "route: clamped to the auto-trade window's {MAX_DISPLAYABLE_STOPS} rows - dropped the last {dropped} stop(s)"
        );
        crate::ffi::notify(&format!("Route clamped to {MAX_DISPLAYABLE_STOPS} stops ({dropped} dropped)"));
    }
    let stop_count = stops.len();
    let route_name = route_ship_name(&stops);

    if write_and_apply_route(ship_index, stops) {
        rename_ship(ship_index, &route_name);
        let action = if append { "appended to" } else { "set on" };
        // Interpolated from the level constants rather than spelled out, so retuning a
        // level cannot leave the report claiming the old letter.
        let verb = match (kind, alt) {
            (RouteKind::Suck, true) => format!("collect at {COLLECT_BUY_LEVEL:?} from (including the no-buy wares)"),
            (RouteKind::Suck, false) => format!("collect at {COLLECT_BUY_LEVEL:?} from"),
            (RouteKind::Trade, true) => format!("trade at {STOP_BUY_LEVEL:?}/{STOP_SELL_LEVEL:?} with (buying everything produced)"),
            (RouteKind::Trade, false) => format!("trade at {STOP_BUY_LEVEL:?}/{STOP_SELL_LEVEL:?} with"),
            // Every fetch stop buys the same list at the same prices, so the list belongs
            // in the verb rather than repeated once per town in `described`.
            (RouteKind::Fetch, every_town) => format!(
                "fetch [{}] (the first stop's own prices) from{}",
                fetch_buys.map(|buys| ware_names(&buys).join(", ")).unwrap_or_default(),
                if every_town { " every town, producer or not," } else { "" }
            ),
            (_, true) => format!("supply {days}-day loads (skipping the low-value inputs) to"),
            (_, false) => format!("supply {days}-day loads to"),
        };
        let lap_note = if scale_to_lap { format!(", {days}-day lap") } else { String::new() };
        info!(
            "route {kind:?} {action} {name:?}: from {load_town_name}, {verb} [{}] (route now {stop_count} stops{lap_note})",
            described.join(", ")
        );
        crate::ffi::notify(&format!(
            "{kind:?} route {action} {route_name}: {} targets from {load_town_name}, {stop_count} stops{lap_note}",
            described.len()
        ));
    }
}

/// The route name for a ship: a leading `-` so generated ships group together when
/// a ship list sorts by name, then the first three letters of each of the route's
/// towns, unique, in route order (e.g. -LueRosSte) - up to ten towns. Capped at 31
/// characters: the ship struct's inline name buffer at +0x160 is 32 bytes and ends
/// the 0x180-stride struct, so anything longer would spill into the next ship. The
/// prefix costs no town: 1 + 10 x 3 is exactly 31.
unsafe fn route_ship_name(stops: &[TradeRouteStop]) -> String {
    let mut route_name = String::from(ROUTE_NAME_PREFIX);
    let mut seen: Vec<u8> = Vec::new();
    for stop in stops {
        if seen.contains(&stop.town_index) {
            continue;
        }
        seen.push(stop.town_index);
        if route_name.chars().count() + 3 > 31 {
            break;
        }
        route_name.push_str(&town_code(stop.town_index).unwrap_or_default());
    }
    route_name
}

/// The marker [route_ship_name] puts at the front of a generated name. It is what
/// [resume_autotrade_after_thaw] uses to tell "a route ship this mod named" from a ship
/// the player named, so the two must agree - hence the shared constant.
pub(crate) const ROUTE_NAME_PREFIX: &str = "-";

/// A town's three-letter code as it appears inside a generated ship name: the first
/// three characters of the town's name (Luebeck -> `Lue`). All 24 town names of the
/// standard map are distinct in their first three characters, so a code identifies one
/// town; a map with two towns sharing a prefix would make [resume_autotrade_after_thaw]
/// treat them as one, which only ever means resuming a ship a little eagerly.
pub(crate) fn town_code(town_index: u8) -> Option<String> {
    let name = get_town_name(town_index)?;
    let code: String = name.chars().take(3).collect();
    (code.chars().count() == 3).then_some(code)
}

/// Rename a ship through the game's rename operations, the way the shipyard does:
/// opcode 0x2d carries the first 12 latin1 bytes, 0x2e chunks append the rest.
unsafe fn rename_ship(ship_index: u16, name: &str) {
    let bytes: Vec<u8> = name.chars().map(|c| if (c as u32) <= 0xff { c as u32 as u8 } else { b'?' }).collect();
    for (i, chunk) in bytes.chunks(12).enumerate() {
        let mut padded = [0u8; 12];
        padded[..chunk.len()].copy_from_slice(chunk);
        let operation = if i == 0 {
            Operation::RenameShip {
                ship_index: ship_index as u32,
                name: padded,
            }
        } else {
            Operation::AppendShipName {
                ship_index: ship_index as u32,
                name: padded,
            }
        };
        OPERATIONS_PTR.enqueue_operation(operation);
    }
}

/// One target town's trade basket, the shape the F4 template repeats: buy what the town
/// produces (at [STOP_BUY_LEVEL]) and sell what it does not (at [STOP_SELL_LEVEL]), all at
/// MAX amounts - the price is the limit here, not a quantity, which is why this needs no
/// consumption reading.
/// With `skip_no_buy` the [NO_BUY_WARES] get no order where the town produces them.
/// Returns (prices, amounts, bought names, skipped names, sold count).
unsafe fn town_trade_basket(
    town_index: u8,
    skip_no_buy: bool,
) -> ([i32; 24], [i32; 24], Vec<String>, Vec<String>, u32) {
    let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
    let mut price = [0i32; 24];
    let mut amount = [0i32; 24];
    let mut bought = Vec::new();
    let mut skipped = Vec::new();
    let mut sold = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        // Buy the town's production minus the NO_BUY_WARES; sell everything else - which
        // includes a NO_BUY_WARE the town produces. A sell order costs nothing: the price
        // is a minimum, so it either trades at a price worth having or does not fire. Give
        // every ware an order rather than reason about which ones could pay off.
        let produced = production[i] > 0;
        if produced && !(skip_no_buy && NO_BUY_WARES.contains(&ware_id)) {
            price[i] = -buy_price(ware_index, STOP_BUY_LEVEL);
            bought.push(format!("{ware_id:?}"));
        } else {
            if produced {
                skipped.push(format!("{ware_id:?}"));
            }
            price[i] = sell_price(ware_index, STOP_SELL_LEVEL);
            sold += 1;
        }
        amount[i] = builder::MAX_AMOUNT;
    }
    (price, amount, bought, skipped, sold)
}

/// The load a supply route must carry for these targets at **current** consumption:
/// the sum of each town's [town_supply_basket] amounts for `days` days of citizen and
/// business consumption, rounded up per town, minus what each target produces itself.
/// `skip_no_supply` mirrors ALT on the route keys; the goods dialog's ALT+F1 and
/// CTRL+ALT+F1 pass false - they set quantities for all goods.
pub(crate) unsafe fn supply_load_for_towns(targets: &[u8], skip_no_supply: bool, days: i32) -> [i32; 24] {
    let mut total = [0i32; 24];
    for &town_index in targets {
        let (load_amount, _, _) = town_supply_basket(town_index, skip_no_supply, days);
        for i in 0..24 {
            total[i] = total[i].saturating_add(load_amount[i]);
        }
    }
    total
}

/// The full-load, full-hull duration of a route in game days, rounded up: the summed
/// travel time of every leg (consecutive stops' towns; a same-town leg is free) plus
/// the 6-hour dwell at each stop (`ship+0x138` acted on at 0x40 = 64 ticks).
/// Calibrated against a sailed lap 30 Aug 2026 - see p3-api's
/// `ShipRoutePtr::calculate_travel_time`. `None` when the router fails a leg.
pub(crate) unsafe fn route_duration_days(stop_towns: &[u8], ship_type: p3_api::data::enums::ShipType) -> Option<u32> {
    const FULL_LOAD_CAPACITY_FACTOR: u32 = 4096 - 614;
    const FULL_HEALTH_FACTOR: u32 = 256;
    const IDLE_TICKS_PER_STOP: u32 = 0x40;
    const TICKS_PER_DAY: u32 = 256;

    let class35 = p3_api::class35::Class35Ptr::new();
    let mut ticks = stop_towns.len() as u32 * IDLE_TICKS_PER_STOP;
    for i in 0..stop_towns.len() {
        let from = stop_towns[i];
        let to = stop_towns[(i + 1) % stop_towns.len()];
        if from == to {
            continue;
        }
        let route = class35.calculate_town_route(GAME_WORLD_PTR.find_town_id(from)?, GAME_WORLD_PTR.find_town_id(to)?)?;
        let time = route.calculate_travel_time(ship_type, FULL_HEALTH_FACTOR, FULL_LOAD_CAPACITY_FACTOR);
        route.free();
        ticks += time;
    }
    Some(ticks.div_ceil(TICKS_PER_DAY))
}

/// One target town's supply basket: `days` days of its citizen and business consumption
/// in raw units, rounded up to whole in-game units, with the R sell prices. Wares the
/// town produces itself are always excluded; with `skip_no_supply` the [NO_SUPPLY_WARES]
/// are too. Returns (amounts, prices, supplied ware count).
///
/// The quantity is citizen **and** business consumption in both cases -
/// `skip_no_supply` narrows which wares are carried, never how much of them.
unsafe fn town_supply_basket(town_index: u8, skip_no_supply: bool, days: i32) -> ([i32; 24], [i32; 24], u32) {
    let town = GAME_WORLD_PTR.get_town(town_index);
    let citizens = town.get_daily_consumptions_citizens();
    let businesses = town.get_daily_consumptions_businesses();
    let production = town.get_production_values();

    let mut load_amount = [0i32; 24];
    let mut sell_prices = [0i32; 24];
    let mut supplied = 0;
    for ware_index in crate::ffi::TRADE_WARES {
        let i = ware_index as usize;
        let ware_id = WareId::from_u16(ware_index).unwrap();
        // A zero load amount is how the route templates express "do not supply this".
        if production[i] > 0 || (skip_no_supply && NO_SUPPLY_WARES.contains(&ware_id)) {
            continue;
        }
        // What the town actually consumes - citizens and businesses - in raw units.
        // Not the t0 threshold: that is a comfortable stock level, inflated by
        // minimum floors and construction reserves, so it would have us ferrying goods
        // that never disappear.
        let period = (citizens[i] + businesses[i]).saturating_mul(days);
        if period == 0 {
            continue; // the town does not consume it
        }
        // Rounded UP to whole in-game units: undersupply empties the office before the
        // ship returns, while the surplus just rides home.
        let scaling = ware_id.get_scaling();
        load_amount[i] = (period + scaling - 1) / scaling * scaling;
        sell_prices[i] = sell_price(ware_index, SUPPLY_SELL_LEVEL);
        supplied += 1;
    }
    (load_amount, sell_prices, supplied)
}

/// What a fetch route collects, read off the buy orders the player left on the ship's own
/// first stop: the maximum price per ware, 0 for a ware the route does not want.
///
/// A buy order is a NEGATED maximum price (see p3-rou's builder), so the sign of the price
/// both identifies a buy and carries its limit - and unlike the amount it is unambiguous,
/// because the game leaves stale positive base prices in slots that carry no instruction
/// but only ever negates a price for a buy. The prices are copied through verbatim, so the
/// first stop is the whole specification: which wares, and what each is worth paying. Set
/// them in the goods dialog (CTRL+Q..Y prices a whole stop, or edit a ware by hand).
fn fetch_buy_prices(first_stop: &TradeRouteStop) -> [i32; 24] {
    let mut prices = [0i32; 24];
    for ware_index in crate::ffi::TRADE_WARES {
        let i = ware_index as usize;
        if first_stop.price[i] < 0 {
            prices[i] = -first_stop.price[i];
        }
    }
    prices
}

/// The wanted wares and the price each will be bought at, for reporting.
fn ware_names(buy_prices: &[i32; 24]) -> Vec<String> {
    crate::ffi::TRADE_WARES
        .filter(|&ware_index| buy_prices[ware_index as usize] > 0)
        .map(|ware_index| format!("{:?}@{}", WareId::from_u16(ware_index).unwrap(), buy_prices[ware_index as usize]))
        .collect()
}

/// The fetch route's targets: every town other than `home` that produces at least one of
/// the wanted wares, or with `every_town` (ALT) simply every town other than `home`.
///
/// Production rather than stock, because this decides where a *standing* route calls: a
/// town that happens to be holding a ware today is not a source to build a circuit around,
/// while a producer keeps refilling between visits. What each stop then *buys* is the whole
/// wanted list either way - see `fetch_buys` in [on_route_hotkey].
///
/// ALT drops the filter for the case the filter gets wrong: a town can hold a wanted ware
/// without producing it - imports, an AI trader's dumping ground, a former producer - and
/// the player's own price limits decide whether anything is actually bought, so a wasted
/// call costs sailing time and nothing else. The price protects the money; production is
/// only a guess at where the goods will be.
unsafe fn fetch_targets(home: u8, buy_prices: &[i32; 24], every_town: bool) -> Vec<u8> {
    let mut towns = Vec::new();
    for town_index in 0..GAME_WORLD_PTR.get_towns_count() as u8 {
        if town_index == home {
            continue;
        }
        let production = GAME_WORLD_PTR.get_town(town_index).get_production_values();
        // into_iter() rather than a bare crate::ffi::TRADE_WARES.any(..): `any` takes &mut self, and
        // calling it straight on a const would silently borrow a temporary copy.
        let produces_wanted = crate::ffi::TRADE_WARES.into_iter().any(|ware_index| {
            let i = ware_index as usize;
            buy_prices[i] > 0 && production[i] > 0
        });
        if every_town || produces_wanted {
            towns.push(town_index);
        }
    }
    towns
}

/// Order the target towns into the shortest closed tour that leaves `home` and comes back
/// to it - the order the route's middle stops are generated in.
///
/// The distances are the game's own: [p3_api::class35::Class35Ptr::town_distance] runs the
/// pathfinder the ships themselves use, so a leg is as long as the water route really is,
/// coastlines and sea lanes included, not a straight line. Travel time divides that
/// distance by a per-SHIP speed factor - the same factor on every leg - so the shortest
/// tour is also the fastest one, whatever ship ends up running it, and the ship's type,
/// hull condition and load never enter the ordering.
///
/// Nearest neighbour from home, then 2-opt until no segment reversal improves the tour.
/// For the handful of towns a ware list produces this is optimal or within a percent of
/// it, and it costs one keypress: 24 towns is 276 router calls and a few hundred
/// reversals. If any distance is unavailable the discovery order is kept - a longer route
/// beats no route.
unsafe fn order_towns_by_distance(home: u8, towns: Vec<u8>) -> Vec<u8> {
    // One target has no order to choose and two are symmetric: home-A-B-home is the same
    // closed tour as home-B-A-home.
    if towns.len() < 3 {
        return towns;
    }

    let mut nodes = Vec::with_capacity(towns.len() + 1);
    nodes.push(home);
    nodes.extend_from_slice(&towns);
    let mut ids = Vec::with_capacity(nodes.len());
    for &town_index in &nodes {
        match GAME_WORLD_PTR.find_town_id(town_index) {
            Some(id) => ids.push(id),
            None => {
                warn!("fetch route: town {town_index} has no town id, keeping the discovery order");
                return towns;
            }
        }
    }

    // The whole distance matrix up front, since 2-opt needs any pair. The router depends
    // on nothing but the two endpoints' static coordinates, so half the matrix is enough
    // and the numbers are the same in every save.
    let n = nodes.len();
    let router = p3_api::class35::Class35Ptr::new();
    let mut distance = vec![0i32; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let Some(d) = router.town_distance(ids[i], ids[j]) else {
                warn!(
                    "fetch route: the router found no route between towns {} and {}, keeping the discovery order",
                    nodes[i], nodes[j]
                );
                return towns;
            };
            distance[i * n + j] = d;
            distance[j * n + i] = d;
        }
    }
    let leg = |a: usize, b: usize| distance[a * n + b];
    let tour_length = |tour: &[usize]| -> i64 { (0..tour.len()).map(|i| leg(tour[i], tour[(i + 1) % tour.len()]) as i64).sum() };

    // Nearest neighbour from home.
    let mut tour = vec![0usize];
    let mut visited = vec![false; n];
    visited[0] = true;
    while tour.len() < n {
        let current = *tour.last().unwrap();
        let next = (1..n).filter(|&j| !visited[j]).min_by_key(|&j| leg(current, j)).unwrap();
        visited[next] = true;
        tour.push(next);
    }
    let greedy_length = tour_length(&tour);

    // 2-opt: reverse any stretch of the tour whose two cut legs get shorter for it. Home
    // stays pinned at position 0 - it is the route's bracket, not a free stop, and the
    // closing leg back to it is accounted for by the wrap in `after`. Every accepted
    // reversal strictly shortens an integer length, so this terminates on its own; the
    // pass cap only guarantees that a defect in the arithmetic cannot hang the game on a
    // keypress.
    for _ in 0..1000 {
        let mut improved = false;
        for i in 1..n - 1 {
            for k in (i + 1)..n {
                let (before, first, last, after) = (tour[i - 1], tour[i], tour[k], tour[(k + 1) % n]);
                if leg(before, last) + leg(first, after) < leg(before, first) + leg(last, after) {
                    tour[i..=k].reverse();
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    let final_length = tour_length(&tour);

    let ordered: Vec<u8> = tour[1..].iter().map(|&i| nodes[i]).collect();
    let names: Vec<String> = ordered
        .iter()
        .map(|&town_index| get_town_name(town_index).unwrap_or_else(|| format!("town {town_index}")))
        .collect();
    debug!(
        "fetch route: {} targets ordered into a closed tour of {final_length} from {} (nearest neighbour alone: {greedy_length}): [{}]",
        ordered.len(),
        get_town_name(home).unwrap_or_else(|| format!("town {home}")),
        names.join(" -> ")
    );
    ordered
}

/// Global pool of applied route stops (VERIFIED in-game): 220-byte records in the .rou
/// stop layout, the first u16 is the next-stop index, the chain is circular. The
/// (lead) ship's ship+0x132 is the CURRENT stop pointer (advances as the route runs);
/// the stop carrying the 0x04 action marker is the route's logical first stop. A head
/// of 0 is ambiguous (pool[0] is a valid index), so empty ship slots also "dump".
pub(crate) const ROUTE_STOP_POOL_COUNT: *const u16 = 0x006dd72a as _;
pub(crate) const ROUTE_STOP_POOL: *const u32 = 0x006dd72c as _;
pub(crate) const ROUTE_STOP_SIZE: u32 = 220;
const SHIP_ROUTE_HEAD_OFFSET: u32 = 0x132;

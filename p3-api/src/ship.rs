use crate::{
    data::{enums::ShipType, p3_ptr::P3Pointer},
    latin1_to_string,
};

pub const SHIP_SIZE: u32 = 0x180;

/// The per-type **minimum sailors to sail**, four bytes indexed by ship type
/// (`ship+0xE & 3`): Snaikka 5, Crayer 8, Cog 10, Holk 12. 26 readers in the exe -
/// among them an AI spawn path that writes it straight into the crew word
/// (`movzx dx,[ecx+0x673660] / mov [esi+0x40],dx` at `0x00519FE5`). The neighbouring
/// table at `0x00673664` (10/16/30/24) is the type's **full crew**, the figure the
/// speed math rewards up to and the hire operation fills toward - not the minimum.
pub const MIN_SAILORS_TABLE_ADDRESS: u32 = 0x00673660;

#[derive(Debug, Clone, Copy)]
pub struct ShipPtr {
    pub address: u32,
}

impl ShipPtr {
    pub fn new(address: u32) -> Self {
        Self { address }
    }

    pub unsafe fn get_next_ship_index_of_merchant(&self) -> u16 {
        self.get(0x04)
    }

    /// The owning merchant (ships chain per owner via +0x4,
    /// `get_next_ship_index_of_merchant`).
    pub fn get_merchant_index(&self) -> u8 {
        unsafe { self.get(0x0) }
    }

    /// Also the link for the ships tick's two ship lists (`ships+0xE8` in port,
    /// `ships+0xEA` at sea): `0x00506720` walks them through this field, so the name
    /// is incomplete - a ship is spliced out of its list with
    /// `[cursor] = ship->0x6; ship->0x6 = 0xFFFF`.
    pub fn get_next_ship_in_convoy(&self) -> u16 {
        unsafe { self.get(0x06) }
    }

    pub fn get_convoy_id(&self) -> u16 {
        unsafe { self.get(0x08) }
    }

    /// Previous ship in the town's docking chain, `0xFFFF` when unlinked. A separate
    /// chain from the per-merchant (`+0x4`) and per-convoy (`+0x6`) ones: it is
    /// ordered by [ShipPtr::get_docking_sort_key], inserted by `0x0050D0C0` and
    /// unlinked by `0x0050D040(ships, index)` - a plain doubly-linked unlink that
    /// writes `0xFFFF` into both links and destroys nothing.
    pub fn get_previous_ship_in_port(&self) -> u16 {
        unsafe { self.get(0x0a) }
    }

    /// Next ship in the town's docking chain, `0xFFFF` when unlinked. See
    /// [ShipPtr::get_previous_ship_in_port].
    pub fn get_next_ship_in_port(&self) -> u16 {
        unsafe { self.get(0x0c) }
    }

    /// The signed key the docking chain is sorted by: `0x0050D0C0` walks the chain
    /// comparing this before linking a ship in. Meaning not yet identified.
    pub fn get_docking_sort_key(&self) -> i16 {
        unsafe { self.get(0x1e) }
    }

    /// The trade-route state byte. **Bit 0 is "automatic trade is running"**: the
    /// [ships tick](`0x00506720`) only executes a route stop for a ship with that bit
    /// set, [crate::operation::Operation::SetTradeRouteActive] writes the whole byte as
    /// `0x01` when activating (`0x0053E0F3`) and clears bits 0 and 1 when deactivating
    /// (`0x0053DF4D`), and a finished stop with action bit `0x02` clears the low bits
    /// again. Bit 0 is also the gate on the "destination port closed" check that turns a
    /// ship away from a frozen or blockaded port.
    pub fn get_trade_route_flags(&self) -> u8 {
        unsafe { self.get(0x136) }
    }

    /// Whether automatic trade is currently running for this ship - bit 0 of
    /// [ShipPtr::get_trade_route_flags].
    pub fn is_trade_route_active(&self) -> bool {
        self.get_trade_route_flags() & 1 != 0
    }

    pub fn get_type(&self) -> ShipType {
        unsafe { self.get(0x0e) }
    }

    pub fn get_capacity(&self) -> u32 {
        unsafe { self.get(0x10) }
    }

    pub fn get_max_health(&self) -> u32 {
        unsafe { self.get(0x14) }
    }

    pub fn get_current_health(&self) -> u32 {
        unsafe { self.get(0x18) }
    }

    pub fn get_x(&self) -> i32 {
        unsafe { self.get(0x1c) }
    }

    pub fn get_y(&self) -> i32 {
        unsafe { self.get(0x20) }
    }

    pub unsafe fn get_destination_town_index(&self) -> u8 {
        self.get(0x38)
    }

    pub fn get_last_town_index(&self) -> Option<u8> {
        let town_index: u8 = unsafe { self.get(0x39) };
        if town_index != 0xff {
            Some(town_index)
        } else {
            None
        }
    }

    pub fn get_wares(&self) -> [i32; 24] {
        unsafe { self.get(0x54) }
    }

    pub fn get_avg_prices(&self) -> [f32; 24] {
        unsafe { self.get(0xb4) }
    }

    pub fn get_payload_buy_sum(&self) -> i32 {
        unsafe { self.get(0x114) }
    }

    /// The ship's 24 artillery slots, one byte each: `0..5` a
    /// [crate::data::enums::ShipWeaponId], `6` the second slot of a large weapon
    /// (whose type byte sits in the even slot before it), `7` a slot this hull does not
    /// have, `0xFF` empty. The weapons operation `0x09` (`0x00538210`) walks exactly
    /// this array to move guns between ship and office.
    pub fn get_artillery_slots(&self) -> [u8; 24] {
        unsafe { self.get(0x13c) }
    }

    /// The cutlasses aboard - the crew's boarding weapons, which are **not** wares and
    /// not artillery: one word right behind the artillery slots, moved by operation
    /// `0x09` with a weapon type of `6` or above, against the office's own count at
    /// `office+0x2BC`.
    pub fn get_cutlasses(&self) -> u16 {
        unsafe { self.get(0x154) }
    }

    /// The ship's captain as an auto-trader index, out-of-range = none. The AI hire
    /// path (0x51a1b9) fills it from the town's tavern captain (resolver 0x5269a0)
    /// and unlinks the record from the town's chain.
    pub fn get_captain_index(&self) -> u16 {
        unsafe { self.get(0x42) }
    }

    pub fn get_status(&self) -> u16 {
        unsafe { self.get(0x134) }
    }

    /// The pool index of the ship's **current** route stop, which advances as the route
    /// runs ([crate::data::route_stop]). Out of range when the ship has no route; the
    /// panel's Auto trade view walks the chain from here.
    pub fn get_route_stop_index(&self) -> u16 {
        unsafe { self.get(0x132) }
    }

    /// The logical first stop of this ship's route, or `None` without one.
    pub unsafe fn get_first_route_stop(&self) -> Option<crate::data::route_stop::RouteStopPtr> {
        crate::data::route_stop::find_first_stop(self.get_route_stop_index())
    }

    /// Set the ship's **current** route stop (`+0x132`) - the stop the route logic
    /// services next. Written as a bare setter in the crate's convention; the game
    /// itself advances this field the same way (`0x00503449` and its siblings) as a
    /// ship finishes each stop.
    pub unsafe fn set_route_stop_index(&self, index: u16) {
        self.set(0x132, &index)
    }

    /// Is the ship at the town named by `get_last_town_index`, rather than out at sea?
    ///
    /// The game classifies its own status field with exactly two tests, next to each
    /// other in the per-merchant ship census at `0x004F0B02`/`0x004F0B11`/`0x004F0B26`:
    /// `status == 0xF` is a merchant vessel at sea (the value mod-scrollmap-render-all-
    /// ships draws, along with `0x12` for an AI pirate), and `status <= 3` is the
    /// in-port family. `0` is lying in the port; `3` is set while entering it
    /// (`0x004E13FA`, which also clears the convoy fields `+0x6`/`+0x8` and ORs `0x60`
    /// into the flags at `+0x3C`), and `+0x39` already names the town then - verified
    /// in-game: a ship sailing to a town flips from `0xF` to `3` at the moment the town
    /// becomes enterable and its tavern reachable, before it has docked.
    pub fn is_in_port(&self) -> bool {
        self.get_status() <= 3
    }

    /// **Docked**: lying at the quay, status `0` exactly - the state the dock function
    /// `0x00519C90` writes, and what the game's own windows require before they will
    /// crew or repair a ship (observed in play). The operations behind them are looser -
    /// the repair thunk tests `status < 4` and the hire-sailors handler `0x00537C20`
    /// tests no status at all - so a mod that drives them wants this test, not theirs.
    ///
    /// The in-port family is four distinct states, each with its own handler in the
    /// ships tick's jump table (`0x00507CB0`, index bytes `0x00507CDC`), and only `0` is
    /// at the quay:
    ///
    /// |Status|Handler|State|
    /// |-|-|-|
    /// |`0`|`0x005067D0`|**lying in port.** The dock function `0x00519C90` writes it, together with the moored flag `0x20` at `+0x3C`, the arrival clock at `+0x44` and `town+0x996 += 1`. Its handler counts idle time at `+0x3E` toward a week (`0x700`)|
    /// |`1`|`0x005068CE`|in the port with a **departure pending** - a countdown at `+0x138`, the current town copied into `+0x37` and the destination into `+0x38` (`0x0050729C`, `0x00509C90`). Its own handler counts idle time like `0`, and the state is also re-entered when the port is closed (`0x0050699D`, after testing the town's frozen/blockade bits `0x04000200`)|
    /// |`2`|`0x00506945`|written at the end of a route stop's ware transfer (`0x00502089`), moored flag re-set at `0x0050209B`; its handler runs a `+0x138` timer and hands over to `1`|
    /// |`3`|`0x00506A86`|**entering the port** (`0x004E13FA`). Its handler counts `+0x138` to `0x40` and then calls the dock function, so `3` becomes `0`|
    ///
    /// A ship the player sees in a town but not at the quay therefore reads `1`, `2` or
    /// `3`; [Self::is_in_port] admits all three, which is why the moored flag `+0x3C` is
    /// no help either - states `0`, `1` and `2` all carry it.
    pub fn is_lying_in_port(&self) -> bool {
        self.get_status() == 0
    }

    /// The crew a convoy's **lead ship** must have: the `cmp [ship+0x40],0x14` at
    /// `0x00519C40`, which is the executable's only test of the crew word against 20.
    pub const CONVOY_LEADER_MIN_CREW: u16 = 20;

    /// The raider status the ships tick dispatches at `0x00507CB0` alongside `0x12` (the AI
    /// pirate's), and the one status the scrollmap's enter-town test skips when it looks
    /// for a ship of the player's in a town (`0x0044A14F`).
    pub const STATUS_RAIDER: u16 = 0x11;

    /// May this ship lead a convoy? The game's own predicate `0x00519C40`, a
    /// `thiscall(ship) -> bool` that reads the record and nothing else:
    ///
    /// |Clause|Meaning|
    /// |-|-|
    /// |`+0x40 >= 0x14`|at least [Self::CONVOY_LEADER_MIN_CREW] sailors|
    /// |`+0x8` past the convoy count -> `+0x18 >= +0x14 / 2`|a ship not already in a convoy needs half its hull|
    /// |`+0x120 >= 0x43`|the weapon strength the battle AI reads as "armed" (`ship_rec+0x120`)|
    /// |`+0x42` inside the auto-trader array|a captain aboard|
    /// |`+0x15C == 0`|meaning not identified|
    ///
    /// The ships tick calls it on a convoy's lead ship before running the convoy's route
    /// stop (`0x00507236`); a `false` files the "trade route is interrupted" note.
    pub unsafe fn can_lead_convoy(&self) -> bool {
        let predicate: extern "thiscall" fn(u32) -> bool = std::mem::transmute(0x0051_9C40u32);
        predicate(self.address)
    }

    /// May this ship sail at all? The game's own predicate `0x00519BA0`, also a
    /// `thiscall(ship) -> bool` over the record alone:
    ///
    /// |Clause|Meaning|
    /// |-|-|
    /// |`+0x40 >= MIN_SAILORS[+0xE & 3]`|the type's minimum crew ([MIN_SAILORS_TABLE_ADDRESS])|
    /// |`+0x3F > 0`|signed byte, meaning not identified|
    /// |`+0x118 <= +0x10`|load within the ship's capacity|
    /// |no captain -> `+0x14 <= 5 * +0x18`|a captainless ship needs a fifth of its hull|
    /// |status not `7`, `9`, `0xC`..`0xE`, or `>= 0x11`|not in a state that forbids sailing|
    /// |the town's flags lack `0x04000200`|the port is neither frozen nor blockaded|
    ///
    /// Called on the lead ship right after [Self::can_lead_convoy] (`0x00507243`), with
    /// the same note as the consequence.
    pub unsafe fn can_sail(&self) -> bool {
        let predicate: extern "thiscall" fn(u32) -> bool = std::mem::transmute(0x0051_9BA0u32);
        predicate(self.address)
    }

    /// Would the game let this ship sail with `crew` sailors aboard - as a convoy's lead
    /// ship when `as_convoy_leader`?
    ///
    /// Both predicates only read the ship record, so this evaluates them against a
    /// **copy** of it with the crew word overwritten: the answer to "is the crew the only
    /// thing stopping it?" without writing a sailor into the live game first.
    pub unsafe fn would_sail_with_crew(&self, crew: u16, as_convoy_leader: bool) -> bool {
        #[repr(align(4))]
        struct Record([u8; SHIP_SIZE as usize]);
        let mut copy = Record([0u8; SHIP_SIZE as usize]);
        std::ptr::copy_nonoverlapping(self.address as *const u8, copy.0.as_mut_ptr(), SHIP_SIZE as usize);
        *(copy.0.as_mut_ptr().add(0x40) as *mut u16) = crew;
        let hypothetical = Self::new(copy.0.as_ptr() as u32);
        (!as_convoy_leader || hypothetical.can_lead_convoy()) && hypothetical.can_sail()
    }

    /// Would the tavern's Sailors page offer to crew this ship? Its list is built by
    /// the ship collector `0x00504AC0` (mode 0, the Sailors page's call at
    /// `0x005D4CA6`), which keeps a ship whose status is `0` or `1` - lying in port, or
    /// holding with a pending departure - and whose trade route is switched off
    /// (`0x00504B59`..`0x00504B6F`). The town match is the caller's business. One state
    /// looser than [Self::is_lying_in_port], the shipyard's rule.
    pub fn is_crewable_in_port(&self) -> bool {
        self.get_status() <= 1 && !self.is_trade_route_active()
    }

    /// A short label for the ship's in-port state, `None` at sea - the table on
    /// [Self::is_lying_in_port] in a form a mod can put in front of the player.
    pub fn get_in_port_state_name(&self) -> Option<&'static str> {
        match self.get_status() {
            0 => Some("lying in port"),
            1 => Some("departure pending"),
            2 => Some("finishing a route stop"),
            3 => Some("entering the port"),
            _ => None,
        }
    }

    /// The crew on board, the word at `+0x40` (summed into the merchant's fleet crew
    /// at `0x004F7E27`, raised by the hire-sailors operation 0x04).
    pub fn get_crew(&self) -> u16 {
        unsafe { self.get(0x40) }
    }

    /// The sailors this ship still wants, as the game computes it (`0x005184F0`,
    /// `thiscall(ship)`): the shortfall below the type's **full crew** (the byte table
    /// at `0x00673664` = 10/16/30/24) plus a cargo-derived term - 1/400 of the figure
    /// `0x005182B0` recomputes into `ship+0x118` by walking the 24 ware amounts at
    /// `+0x54` with the barrels/loads scale table `0x00672C14` (its exact meaning is
    /// unverified; armament plays no part anywhere here) - capped by the remaining room
    /// (`ship+0xF x type_static + capacity/1000`, floored at 20, minus the crew).
    /// `ship+0xF` is plausibly the build grade a shipyard's experience sets (better
    /// yards produce ships with more capacity) - unverified.
    ///
    /// This routine is the authority on "how many sailors can this ship take": the
    /// tavern's Sailors page clamps its input field with exactly it (two calls at
    /// `0x005D517C`/`0x005D518F`, overwriting an entry that exceeds it), and the hire
    /// operation's handler re-clamps every request through it. This
    /// is the figure the hire-sailors handler (opcode 0x04, `0x00537C20`) clamps a
    /// request to - so a tavern hire fills toward *full performance*, not toward the
    /// bare sailing minimum ([Self::get_min_sailors]); `<= 0` means nothing wanted.
    pub fn get_free_sailor_berths(&self) -> i32 {
        let free_berths: extern "thiscall" fn(u32) -> i32 = unsafe { std::mem::transmute(0x005184F0u32) };
        free_berths(self.address)
    }

    /// The minimum sailors this ship needs to sail at all: the per-type byte table at
    /// [MIN_SAILORS_TABLE_ADDRESS], indexed by `ship+0xE & 3`. Snaikka 5, Crayer 8,
    /// Cog 10, Holk 12 - in-game verified 30 Aug 2026, and independent of upgrade
    /// level. Read from the game's own table rather than hardcoded.
    pub fn get_min_sailors(&self) -> u8 {
        let type_index: u8 = unsafe { self.get::<u8>(0x0e) } & 3;
        unsafe { *((MIN_SAILORS_TABLE_ADDRESS + type_index as u32) as *const u8) }
    }

    pub fn get_name(&self) -> String {
        let buf: [u8; 32] = unsafe { self.get(0x160) };
        latin1_to_string(&buf)
    }

    pub unsafe fn calc_free_capacity(&self) -> i32 {
        //TODO weapons, sailors
        self.get_capacity() as i32 - self.get_wares().iter().sum::<i32>() - 10000
    }
}

impl P3Pointer for ShipPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

//! The sea-battle simulation: the battle pool, the battle object and the per-ship battle
//! object the engine steps once per ship per tick.
//!
//! An auto-resolved battle runs the same tick-by-tick simulation as the one the player
//! enters - the same stepper, the same per-ship AI - so anything applied to a battle ship
//! object during the fight is executed by the engine either way.
//!
//! Static analysis of `Patrician3.exe` (`.claude/notes/todo/battle-orders-force-flee.md`),
//! with the flee operation and the battle-map coordinates confirmed by the operation logger.

use crate::data::p3_ptr::P3Pointer;

/// The battle pool at `0x006E55D0`: up to [BATTLE_POOL_SLOTS] battle-object pointers, a free
/// slot holding 0. The constructor stores the new object at `[pool + slot*4]` and the slot
/// into the object's `+0x662` (`0x00603A97`..`0x00603AAA`); `0x00611CD0(this = pool)` returns
/// the first free slot, or `0xFF` when the pool is full.
pub const BATTLE_POOL_ADDRESS: u32 = 0x006E55D0;
pub const BATTLE_POOL_SLOTS: u32 = 0xFF;
/// `[0x006E59D4]`: the number of live battles.
pub const BATTLE_COUNT_ADDRESS: u32 = 0x006E59D4;
/// `[0x006E59CC]`, a byte: the pool slot of the battle **the player has entered**, or
/// [NO_BATTLE_SLOT]. Operation `0x96` (enter / decline) toggles it against the battle's own
/// slot at `0x0060FEFE`..`0x0060FF1E`; the battle-over paths reset it to `0xFF`.
pub const PLAYER_BATTLE_SLOT_ADDRESS: u32 = 0x006E59CC;
pub const NO_BATTLE_SLOT: u8 = 0xFF;

/// The pool of running battles.
#[derive(Debug, Clone, Copy, Default)]
pub struct BattlePoolPtr;

impl BattlePoolPtr {
    pub const fn new() -> Self {
        Self
    }

    /// The battle in `slot`, if one is running there.
    pub unsafe fn get_battle(&self, slot: u8) -> Option<BattlePtr> {
        if slot as u32 >= BATTLE_POOL_SLOTS {
            return None;
        }
        let address = *((BATTLE_POOL_ADDRESS + slot as u32 * 4) as *const u32);
        (address != 0).then_some(BattlePtr::new(address))
    }

    pub unsafe fn count(&self) -> u32 {
        *(BATTLE_COUNT_ADDRESS as *const u32)
    }

    /// The slot of the battle the player has entered, [NO_BATTLE_SLOT] when none.
    pub unsafe fn player_battle_slot(&self) -> u8 {
        *(PLAYER_BATTLE_SLOT_ADDRESS as *const u8)
    }
}

/// One running battle. Two vtables exist for the same layout: the interactive battle's
/// (`0x0067AAA0`, its stepper the heavy `0x005FD9xx`) and the record battle's
/// (`0x0067AD44`, stepper `0x00606CC0`), twelve slots each, slot 5 the per-step update.
#[derive(Debug, Clone, Copy)]
pub struct BattlePtr {
    pub address: u32,
}

impl BattlePtr {
    pub const VTABLE_INTERACTIVE: u32 = 0x0067AAA0;
    pub const VTABLE_RECORD: u32 = 0x0067AD44;
    /// `+0x15C` reads this once the battle is decided (`0x0060788F`).
    pub const STATUS_OVER: u16 = 0xFFFE;

    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// The array of [BattleShipPtr] pointers at `+0x4`, indexed by battle-ship index.
    pub unsafe fn get_ship_object(&self, index: u16) -> Option<BattleShipPtr> {
        let array: u32 = self.get(0x4);
        if array == 0 {
            return None;
        }
        let address = *((array + index as u32 * 4) as *const u32);
        (address != 0).then_some(BattleShipPtr::new(address))
    }

    /// Head of a side's ship chain: side 0 at `+0x10`, side 1 at `+0x65C`, linked through
    /// the word array at `+0xC`.
    pub unsafe fn get_side_head(&self, side: u8) -> u16 {
        self.get(if side == 0 { 0x10 } else { 0x65C })
    }

    /// The byte the stepper tests at `0x006074C0` and `0x00607878`; written `1` on the
    /// battle-start paths (`0x00606C36`, `0x0060ECA9`, ..) and `0` at `0x005FD077`,
    /// `0x00606428`.
    pub unsafe fn get_marker(&self) -> u8 {
        self.get(0x5C)
    }

    /// The countdown / status word at `+0x15C`; [Self::STATUS_OVER] ends the battle.
    pub unsafe fn get_status(&self) -> u16 {
        self.get(0x15C)
    }

    /// This battle's own pool slot (`+0x662`, written by the constructor).
    pub unsafe fn get_slot(&self) -> u16 {
        self.get(0x662)
    }

    /// Whether this is the battle the player has entered: its slot equals
    /// `[`[PLAYER_BATTLE_SLOT_ADDRESS]`]`, exactly the comparison operation `0x96` makes.
    pub unsafe fn is_player_battle(&self) -> bool {
        let slot = self.get_slot();
        slot <= 0xFF && slot as u8 == BattlePoolPtr::new().player_battle_slot()
    }

    /// The byte handed to every per-ship step (`+0x670`).
    pub unsafe fn get_step_argument(&self) -> u8 {
        self.get(0x670)
    }

    /// Step counter `+0xA8C`, 0..24.
    pub unsafe fn get_step_counter(&self) -> u8 {
        self.get(0xA8C)
    }

    /// Side aggregates, recomputed every step (`0x00607258`..`0x00607484`), indexed by
    /// literal side 0/1. Ships out of the fight ([BattleShipPtr::OUT_OF_FIGHT_MASK]) are
    /// skipped.
    pub unsafe fn get_side_gunnery(&self, side: u8) -> u32 {
        self.get(0xA90 + side as u32 * 4)
    }

    pub unsafe fn get_side_max_crew(&self, side: u8) -> u16 {
        self.get(0xA98 + side as u32 * 2)
    }

    /// `crew < cutlasses ? crew * 2 : crew + cutlasses`, the maximum over the side.
    pub unsafe fn get_side_max_boarding_power(&self, side: u8) -> u16 {
        self.get(0xA9C + side as u32 * 2)
    }

    /// Maximum `ship+0x120` (the armament figure) over the side.
    pub unsafe fn get_side_max_armament(&self, side: u8) -> u32 {
        self.get(0xAA0 + side as u32 * 4)
    }

    /// Maximum speed (`0x00612930`) over the side.
    pub unsafe fn get_side_max_speed(&self, side: u8) -> u32 {
        self.get(0xAA8 + side as u32 * 4)
    }

    /// The per-side scale factor of the flee tests (`+0xAB0`), written 1 at battle start.
    pub unsafe fn get_side_scale(&self, side: u8) -> u8 {
        self.get(0xAB0 + side as u32)
    }

    /// Mean battle-map position of the side's live ships; `0x1100` / `0xF02` (record) or
    /// `0x880` / `0x781` (interactive) when the side has none left.
    pub unsafe fn get_side_mean_x(&self, side: u8) -> i32 {
        self.get(0xAB4 + side as u32 * 4)
    }

    pub unsafe fn get_side_mean_y(&self, side: u8) -> i32 {
        self.get(0xABC + side as u32 * 4)
    }

    /// Pending outcome: `0xFF` none, `0` sunk or taken, `2` plundered; `1` and `3` not decoded.
    pub unsafe fn get_outcome_kind(&self) -> u8 {
        self.get(0xAC4)
    }

    /// The ship index the outcome refers to.
    pub unsafe fn get_outcome_ship(&self) -> u16 {
        self.get(0xAC6)
    }

    /// The **panic latch** at `+0xAC9`: every flee branch of the per-ship AI sets it, and
    /// while it is clear an undamaged ship skips the fight-or-flee evaluation altogether
    /// (`0x00622804`: hull unchanged since battle start **and** latch clear -> stand and
    /// fight).
    pub unsafe fn get_panic_latch(&self) -> bool {
        self.get::<u8>(0xAC9) != 0
    }

    pub unsafe fn set_panic_latch(&self, set: bool) {
        self.set(0xAC9, &(set as u8))
    }
}

impl P3Pointer for BattlePtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

/// One ship inside a battle. Constructed at `0x0061A4F0`..`0x0061A5C0`; its vtable's slot 0
/// is the per-ship AI step the stepper calls once per ship per tick
/// (`0x006075D9` / `0x006076B9`: `call [vtable+0]` with the battle's `+0x670` byte).
#[derive(Debug, Clone, Copy)]
pub struct BattleShipPtr {
    pub address: u32,
}

impl BattleShipPtr {
    /// The two vtables, **module-relative** for hooklet's `hook_function_pointer`
    /// (`0x0067AAD0` and `0x0067B0D4`).
    pub const VTABLE_MAIN_OFFSET: u32 = 0x0027AAD0;
    pub const VTABLE_TWIN_OFFSET: u32 = 0x0027B0D4;
    /// Slot 0 of both vtables: the per-ship AI step, `thiscall(this, step_argument)`,
    /// `ret 4`. The two functions are twins (`0x00621770`, `0x0061A9E0`); which one a ship
    /// runs depends on the vtable its object was constructed with.
    pub const SLOT_STEP: u32 = 0;
    pub const STEP_MAIN_ADDRESS: u32 = 0x00621770;
    pub const STEP_TWIN_ADDRESS: u32 = 0x0061A9E0;

    /// `+0x12A` state flags.
    /// Fleeing - written by the AI's own decision every step and by operation `0x9B`.
    pub const FLAG_FLEEING: u8 = 0x01;
    /// Sunk / out (`0x006219F0`, `0x006077BD`, `0x00607855`).
    pub const FLAG_SUNK: u8 = 0x02;
    /// Captured (`0x0060CA7F`..`0x0060CD55`).
    pub const FLAG_CAPTURED: u8 = 0x04;
    /// Grappled / boarding, on both ships (`0x00609CDA`, `0x00609CEE`).
    pub const FLAG_GRAPPLED: u8 = 0x08;
    /// Firing / aim state (`0x00623EEA` set, `0x00624403` cleared).
    pub const FLAG_FIRING: u8 = 0x10;
    /// Disengaged, out of the fight - written only by operation `0x9A`.
    pub const FLAG_DISENGAGED: u8 = 0x20;
    /// Read as "carrying goods" by the plunder paths; no writer found.
    pub const FLAG_PLUNDER_WORTHY: u8 = 0x40;
    /// Modifier on the `+0x13E` order mode (operation `0x97`).
    pub const FLAG_ORDER_MODIFIER: u8 = 0x80;
    /// A ship carrying any of these no longer takes part in the fight.
    pub const OUT_OF_FIGHT_MASK: u8 = Self::FLAG_SUNK | Self::FLAG_CAPTURED | Self::FLAG_DISENGAGED;
    /// What operation `0x9B` keeps of the flags when it orders a flee.
    pub const FLEE_KEEP_MASK: u8 = 0x7E;

    pub fn new(address: u32) -> Self {
        Self { address }
    }

    /// The world ship index (`+0x4`), into the [crate::ships::ShipsPtr] array.
    pub unsafe fn get_ship_index(&self) -> u32 {
        self.get(0x4)
    }

    /// Battle-map position, signed words at `+0xA` / `+0xE`.
    pub unsafe fn get_x(&self) -> i16 {
        self.get(0xA)
    }

    pub unsafe fn get_y(&self) -> i16 {
        self.get(0xE)
    }

    /// Heading 0..31; `(h + 0x10) & 0x1F` is the reciprocal.
    pub unsafe fn get_heading(&self) -> u8 {
        self.get(0x11)
    }

    /// Sail / speed setting, 4 after construction.
    pub unsafe fn get_sail_setting(&self) -> u8 {
        self.get(0x26)
    }

    /// Target or boarding partner, `0xFFFF` when none.
    pub unsafe fn get_target(&self) -> u16 {
        self.get(0x128)
    }

    pub unsafe fn get_flags(&self) -> u8 {
        self.get(0x12A)
    }

    pub unsafe fn set_flags(&self, flags: u8) {
        self.set(0x12A, &flags)
    }

    /// The battle this ship object belongs to (`+0x12C`).
    pub unsafe fn get_battle(&self) -> Option<BattlePtr> {
        let address: u32 = self.get(0x12C);
        (address != 0).then_some(BattlePtr::new(address))
    }

    /// Side index, 0 or 1 (`+0x13D`).
    pub unsafe fn get_side(&self) -> u8 {
        self.get(0x13D)
    }

    /// Order mode byte (`+0x13E`), 0 = none; operation `0x97` sets it, `0x9A`/`0x9B` clear it.
    pub unsafe fn get_order_mode(&self) -> u8 {
        self.get(0x13E)
    }

    pub unsafe fn set_order_mode(&self, mode: u8) {
        self.set(0x13E, &mode)
    }

    /// Crew (`ship+0x40`) and hull (`ship+0x18`) captured at battle start.
    pub unsafe fn get_crew_at_start(&self) -> u16 {
        self.get(0x140)
    }

    pub unsafe fn get_hull_at_start(&self) -> u32 {
        self.get(0x144)
    }

    /// The applied sail setting (`+0x148`) and its cooldown (`+0x149`, `0x4B`).
    pub unsafe fn get_applied_sail_setting(&self) -> u8 {
        self.get(0x148)
    }

    /// Boarding role (`+0x14A`), 1 after construction.
    pub unsafe fn get_boarding_role(&self) -> u8 {
        self.get(0x14A)
    }

    /// Whether the ship is still part of the fight.
    pub unsafe fn is_in_fight(&self) -> bool {
        self.get_flags() & Self::OUT_OF_FIGHT_MASK == 0
    }

    /// Exactly what operation `0x9B` (`0x00541700`) does to a ship when the flee button
    /// is pressed: keep [Self::FLEE_KEEP_MASK] of the flags, set [Self::FLAG_FLEEING], and
    /// drop any `+0x13E` order. The per-ship AI re-decides the flag every step, so a single
    /// call lasts one step at most.
    pub unsafe fn order_flee(&self) {
        self.set_flags((self.get_flags() & Self::FLEE_KEEP_MASK) | Self::FLAG_FLEEING);
        self.set_order_mode(0);
    }
}

impl P3Pointer for BattleShipPtr {
    fn get_address(&self) -> u32 {
        self.address
    }
}

use crate::data::enums::WareId;

/// The weapon type that means "cutlasses" to [Operation::ShipMoveWeapons]: the handler
/// treats anything from 6 up as cutlasses, artillery being 0..5.
pub const CUTLASS_WEAPON_TYPE: u32 = 6;

#[derive(Debug)]
pub enum Operation {
    MoveShipToTown {
        ship_index: u32,
        town_index: u8,
    },
    ShipSellWares {
        amount: i32,
        ware_id: WareId,
        ship_index: u16,
        merchant_index: u16,
        town_index: u16,
    },
    ShipBuyWares {
        amount: i32,
        ware_id: WareId,
        ship_index: u16,
        merchant_index: u16,
        town_index: u16,
    },
    RepairShip {
        ship_index: u32,
    },
    /// Hire sailors from the ship's town's tavern (opcode 0x04, handler 0x00537C20).
    ///
    /// The handler resolves the town from the ship's own `+0x39` and does all the
    /// clamping itself: the request is reduced to the crew the ship can still take
    /// (`0x005184F0`) and to the owner's available sailors in that town (`0x004F6CA0`,
    /// see `MerchantPtr::get_available_sailors`), then the sailor pool is drawn down,
    /// the crew word at `ship+0x40` raised, and the town's beggar pool reduced. So a
    /// generous count simply hires "as many as possible".
    HireSailors {
        ship_index: u32,
        count: u32,
    },
    /// Dismiss sailors into the ship's town (opcode 0x05, handler 0x00537DD0).
    ///
    /// Guards: the ship must have crew and be in port (`ship+0x134 < 4`). The dismissed
    /// sailors rejoin the town as beggars (`town+0x2E4`) and citizens (`town+0x2D4`).
    /// A partial dismissal costs crew morale - `ship+0x3E` drops by
    /// `count*2560/(crew+1)`, floored at 0 - while dismissing everyone (count >= crew)
    /// zeroes crew and morale outright. Both paths recompute the cargo figures
    /// (0x005182B0) and refresh (0x00517770).
    DismissSailors {
        ship_index: u32,
        count: u32,
    },
    ShipMoveWares {
        amount: i32,
        ship_index: u16,
        ware_id: WareId,
        merchant_index: u16,
        to_ship: bool,
    },
    /// Move artillery or cutlasses between a ship and the owner's office in the town it
    /// lies in (opcode 0x09, handler 0x00538210).
    ///
    /// `weapon_type` is a [crate::data::enums::ShipWeaponId] for artillery, or
    /// [CUTLASS_WEAPON_TYPE] and above for the crew's cutlasses. `amount` counts guns
    /// (not slots: a large weapon holds two slots but is one gun) or cutlasses.
    ///
    /// Guards: the town must be the ship's own (`ship+0x39`), the ship must be in port
    /// (`ship+0x134 < 4`), and the owner must have an office there. A ship that is its
    /// **convoy's lead is refused** unless the convoy carries flag `0x2`. Unloading
    /// credits `office+0x124 + type*4` for artillery and `office+0x2BC` for cutlasses;
    /// loading is clamped to the office's stock and, for cutlasses, to a tenth of the
    /// ship's free capacity.
    ShipMoveWeapons {
        weapon_type: u32,
        town_index: u32,
        ship_index: u32,
        amount: i32,
        to_ship: bool,
    },
    MoveWaresConvoy {
        raw_amount: i32,
        convoy_index: u16,
        ware: WareId,
        merchant_index: u16,
        to_ship: bool,
    },
    RepairConvoy {
        convoy_index: u32,
    },
    OfficeAutotradeSettingChange {
        stock: i32,
        price: i32,
        office_index: u32,
        ware_id: WareId,
    },
    /// Employ an administrator in the merchant's office in that town (opcode 0x5E, mode 0,
    /// handler 0x0053D990). The handler resolves the office by merchant and town
    /// (0x005308A0) and, if `office+0x2F2` holds no live auto-trader index, allocates a
    /// fresh record (0x005097C0), stores its index there, zeroes its trade skill and
    /// recomputes the wage (0x004FE160). An office that already has one is left alone.
    /// The trading office window issues it from its hire button at 0x005DC945.
    HireAdministrator {
        merchant_index: u16,
        town_index: u16,
    },
    /// Dismiss the office's administrator (opcode 0x5E, mode 1): frees his record to
    /// the freelist (0x005098B0) and clears bits 0-1 of `office+0x2D6`. A re-hire gets a
    /// new record at trade 0.
    DismissAdministrator {
        merchant_index: u16,
        town_index: u16,
    },
    /// The administrator view's per-ware checkbox "Lock min. store quantity for auto
    /// trade ships": sets or clears the ware's bit in the office lock bitmap
    /// (office+0x3b4). The executor (opcode 0x66, handlers 0x53644b/0x53dd90) resolves
    /// the office by town and merchant.
    OfficeAutotradeLockChange {
        ware_id: WareId,
        town_index: u16,
        merchant_index: u16,
        lock: bool,
    },
    /// Activate or deactivate a ship's trade route: the route panel's "active"
    /// checkbox (opcode 0x68, executor 0x53df00). transfer_loaded_traderoute enqueues
    /// the deactivation as its first step when replacing a route (0x5494dd).
    SetTradeRouteActive {
        ship_index: u32,
        active: bool,
    },
    /// Remove one stop from an applied trade route: what selecting the town "none" in
    /// the route panel enqueues. Opcode 0x6a with town 0xff (executor 0x53e610): frees
    /// the stop's pool record (via the pool free 0x4d4e80), moves the first-stop marker
    /// to the successor if needed and retargets ships heading for the removed stop
    /// (0x509030). Verified in-game: pool indices are reused, so the record identity
    /// comes from the live chain, never from positions.
    RemoveTradeRouteStop {
        stop_pool_index: u32,
        ship_index: u32,
    },
    /// Insert a stop into an applied trade route after an existing stop: selecting a
    /// town in the route panel. The same opcode 0x6a with the +0x8 flag set: allocates
    /// a pool record (via the pool alloc 0x4d4c90), links it after
    /// `after_stop_pool_index` and sets its town.
    InsertTradeRouteStop {
        after_stop_pool_index: u32,
        town_index: u8,
        ship_index: u32,
    },
    /// Rename a ship: the shipyard's "change name" button. Opcode 0x2d (switch case
    /// 0x535caa, handler 0x53cce0 - shared with [Operation::AppendShipName]): copies
    /// exactly 12 name bytes from +0x4 (forcing a NUL after them), bounds-checks the
    /// ship index at +0x10, and assigns the name through the dynamic-name registry
    /// (0x512d40, this = 0x6ddaa0) targeting ship+0x15e (the ship's registry id
    /// word) - which keeps both the registry (used by letter texts) and the ship's
    /// inline name at +0x160 in sync. Longer names are sent as 0x2d followed by
    /// 0x2e chunks (the shipyard sender 0x5faf5d chunks by 12; its text field allows
    /// 15 characters total).
    RenameShip {
        ship_index: u32,
        /// The new name's first chunk: latin1, up to 12 bytes, NUL-padded.
        name: [u8; 12],
    },
    /// Append to a ship's name: the continuation chunk the shipyard sends for names
    /// longer than 12 characters. Opcode 0x2e, same layout and handler as
    /// [Operation::RenameShip], but the handler appends the chunk to the ship's
    /// existing registry name instead of replacing it.
    AppendShipName {
        ship_index: u32,
        /// The next name chunk: latin1, up to 12 bytes, NUL-padded.
        name: [u8; 12],
    },
    /// Sets the game speed. Opcode 0xC8 is handled inline by the operation queue's
    /// drain (handler `0x00546DCF`, jump table `0x00547290`), not by the operation
    /// switch: `+0x4` sets the ms-per-tick divisor of speed level 1 (`ops+0x8D4`),
    /// `+0x8` the divisor of level 2 (`ops+0x8D8`), `+0xC` the level itself
    /// (`ops+0x92C`) and `+0x10` `ops+0x91C`. A field of `-1` leaves that setting
    /// unchanged.
    ///
    /// The levels, from the tick pacer at `0x00546620` (it converts elapsed real
    /// milliseconds into an advance-time operation, opcode 0xC4). The dispatch is
    /// `sub eax,0 / je` then `dec eax / je` at `0x0054674A`: level 0 = normal play,
    /// divisor `ops+0x8D4`, at most 8 ticks per batch; level 1 = fast forward,
    /// divisor `ops+0x8D8`, up to 256 ticks (a day); level 2 = local map, one tick
    /// per `[0x673CF8]` = 3375 ms and at most 1 per batch, which is why the speed
    /// controls do nothing in town view or a sea battle. `ops+0x914` is the master
    /// run flag the pacer requires; the handler sets it to 1 unless a network round
    /// is pending.
    ///
    /// The scrollmap's speed buttons enqueue exactly this: `0x004202A0` with level 0,
    /// `0x00420300` with level 1, both leaving the divisors at `-1`.
    /// Opcode 0x45 (`0x00535FA4`): stores `stands` into the merchant's candidature flag at
    /// `+0x118` via `0x004F9390` (a human's value is taken as given; an AI merchant's is
    /// forced to 1). The town hall's "Accept candidature?" Yes/No.
    SetCandidature {
        merchant_index: u32,
        stands: bool,
    },
    /// Opcode 0x46 (handler `0x00535FD6`, the dispatcher's switch case): make
    /// `merchant_index` mayor of `town_index`. Refused unless the seat is not held by a merchant (`town+0x6F1 >=
    /// merchant count`) and the merchant is at least [crate::merchant::RANK_PATRICIAN] in
    /// his hometown - or `+0x10` carries `0xBADEAFFE`, which waives the rank check only;
    /// `cheat` sets it, as do the 14-day vacancy task 0x2F (`0x004EA1D9`) and the debug
    /// menu's "make mayor" commands.
    AppointMayor {
        merchant_index: u32,
        town_index: u32,
        cheat: bool,
    },
    /// The sea-battle orders, opcodes `0x93`..`0x9B`: each addresses the battle in pool slot
    /// `battle_slot` ([crate::battle::BattlePoolPtr]) and a **set of its ships** - `group`
    /// picks a bank of 64 battle-ship indices, `mask_lo` bits 0..31 the indices
    /// `group*64 + 0..31`, `mask_hi` the indices `group*64 + 32..63`. Dispatched through
    /// the operation switch's 9-byte inline stubs into `0x0054xxxx`.
    ///
    /// Opcode 0x93 (`0x00541050`): set each ship's target (`+0x128`) unless it is grappled,
    /// and clear its flee flag.
    BattleSetTarget {
        battle_slot: u8,
        group: u8,
        target: u16,
        mask_hi: u32,
        mask_lo: u32,
    },
    /// Opcode 0x95 (`0x00541200`): move to a battle-map point (`0x006270C7` on the ship
    /// object), drop the target and **clear the flee flag** - which is why steering by hand
    /// cancels a flee order. `x`/`y` are in the range of the ship objects' `+0xA`/`+0xE`.
    BattleMoveTo {
        battle_slot: u8,
        group: u8,
        x: u16,
        y: u16,
        mask_hi: u32,
        mask_lo: u32,
    },
    /// Opcode 0x97 (`0x00541380`): set the `+0x13E` order mode; `modifier` sets flag `0x80`;
    /// a non-zero mode clears the flee flag. The two aggressive modes are not named.
    BattleSetOrderMode {
        battle_slot: u8,
        mode: u8,
        modifier: bool,
        group: u8,
        mask_hi: u32,
        mask_lo: u32,
    },
    /// Opcode 0x98 (`0x005414E0`): set the sail setting (`+0x148`).
    BattleSetSail {
        battle_slot: u8,
        value: u8,
        group: u8,
        mask_hi: u32,
        mask_lo: u32,
    },
    /// Opcode 0x9A (`0x005415E0`): clear the order mode and mark the ship out of the fight
    /// (`+0x12A = (f & 0x5E) | 0x20`). Not flee; surrender is the open candidate.
    BattleDisengage {
        battle_slot: u8,
        group: u8,
        mask_hi: u32,
        mask_lo: u32,
    },
    /// Opcode 0x9B (`0x00541700`): **flee** - `flee` sets `+0x12A = (f & 0x7E) | 0x01` and
    /// `+0x13E = 0`, cleared with `f & 0xFE`. What the battle window's flee button sends,
    /// confirmed with the operation logger. The per-ship AI overwrites the flag on its next
    /// step, so the order lasts one step unless something re-asserts it.
    BattleFlee {
        battle_slot: u8,
        flee: bool,
        group: u8,
        mask_hi: u32,
        mask_lo: u32,
    },
    SetGameSpeed {
        speed1_ms_per_tick: i32,
        speed2_ms_per_tick: i32,
        level: i32,
        field_10: i32,
    },
}

impl Operation {
    pub fn to_raw(&self) -> [u8; 0x14] {
        let mut op: [u8; 0x14] = [0; 0x14];
        match self {
            Operation::MoveShipToTown { ship_index, town_index } => {
                op[0x04..0x08].copy_from_slice(&ship_index.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&(*town_index as u32).to_le_bytes());
            }
            Operation::ShipSellWares {
                amount,
                ware_id,
                ship_index,
                merchant_index: field_8,
                town_index,
            } => {
                let opcode: u32 = 0x01;
                op[0x00..0x04].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&amount.to_le_bytes());
                op[0x08..0x0a].copy_from_slice(&(*ware_id as u16).to_le_bytes());
                op[0x0a..0x0c].copy_from_slice(&ship_index.to_le_bytes());
                op[0x0c..0x0e].copy_from_slice(&field_8.to_le_bytes());
                op[0x0e..0x10].copy_from_slice(&town_index.to_le_bytes());
            }
            Operation::ShipBuyWares {
                amount,
                ware_id,
                ship_index,
                merchant_index: field_8,
                town_index,
            } => {
                let opcode: u32 = 0x02;
                op[0x00..0x04].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&amount.to_le_bytes());
                op[0x08..0x0a].copy_from_slice(&(*ware_id as u16).to_le_bytes());
                op[0x0a..0x0c].copy_from_slice(&ship_index.to_le_bytes());
                op[0x0c..0x0e].copy_from_slice(&field_8.to_le_bytes());
                op[0x0e..0x10].copy_from_slice(&town_index.to_le_bytes());
            }
            Operation::RepairShip { ship_index: ship_id } => {
                let opcode: u32 = 0x03;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&ship_id.to_le_bytes());
            }
            Operation::HireSailors { ship_index, count } => {
                let opcode: u32 = 0x04;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&ship_index.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&count.to_le_bytes());
            }
            Operation::DismissSailors { ship_index, count } => {
                let opcode: u32 = 0x05;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&ship_index.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&count.to_le_bytes());
            }
            Operation::ShipMoveWares {
                amount,
                ship_index,
                ware_id,
                merchant_index,
                to_ship,
            } => {
                let opcode: u32 = 0x08;
                let ware_id: u16 = *ware_id as _;
                op[0x00..0x04].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&amount.to_le_bytes());
                op[0x08..0x0a].copy_from_slice(&ship_index.to_le_bytes());
                op[0x0a..0x0c].copy_from_slice(&ware_id.to_le_bytes());
                op[0x0c..0x0e].copy_from_slice(&merchant_index.to_le_bytes());
                op[0x0e] = *to_ship as u8;
            }
            Operation::ShipMoveWeapons {
                weapon_type,
                town_index,
                ship_index,
                amount,
                to_ship,
            } => {
                let opcode: u32 = 0x09;
                // The direction rides in the sign bit of the town field: set means
                // ship -> office (0x005382A1 tests for exactly 0x80000000).
                let town = if *to_ship { *town_index } else { town_index | 0x8000_0000 };
                op[0x00..0x04].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&weapon_type.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&town.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&ship_index.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&amount.to_le_bytes());
            }
            Operation::MoveWaresConvoy {
                raw_amount: amount,
                convoy_index: convoy_id,
                ware,
                merchant_index: merchant_id,
                to_ship,
            } => {
                let opcode: u32 = 0x1b;
                let ware_id: u16 = *ware as _;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4..8].copy_from_slice(&amount.to_le_bytes());
                op[8..0x0a].copy_from_slice(&convoy_id.to_le_bytes());
                op[0x0a..0x0c].copy_from_slice(&ware_id.to_le_bytes());
                op[0x0c..0x0e].copy_from_slice(&merchant_id.to_le_bytes());
                if *to_ship {
                    op[0x0e] = 1;
                } else {
                    op[0x0e] = 0;
                }
            }
            Operation::RepairConvoy { convoy_index: convoy_id } => {
                let opcode: u32 = 0x1d;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&convoy_id.to_le_bytes());
            }
            Operation::OfficeAutotradeSettingChange {
                stock,
                price,
                office_index,
                ware_id,
            } => {
                let opcode: u32 = 0x5b;
                let ware_id: u32 = *ware_id as _;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4..8].copy_from_slice(&stock.to_le_bytes());
                op[8..0x0c].copy_from_slice(&price.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&office_index.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&ware_id.to_le_bytes());
            }
            Operation::SetCandidature { merchant_index, stands } => {
                let opcode: u32 = 0x45;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4..8].copy_from_slice(&merchant_index.to_le_bytes());
                op[8..0x0c].copy_from_slice(&(*stands as u32).to_le_bytes());
            }
            Operation::AppointMayor {
                merchant_index,
                town_index,
                cheat,
            } => {
                let opcode: u32 = 0x46;
                let token: u32 = if *cheat { 0xBADE_AFFE } else { 0 };
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4..8].copy_from_slice(&merchant_index.to_le_bytes());
                op[8..0x0c].copy_from_slice(&town_index.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&token.to_le_bytes());
            }
            Operation::BattleSetTarget {
                battle_slot,
                group,
                target,
                mask_hi,
                mask_lo,
            } => {
                let opcode: u32 = 0x93;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4] = *battle_slot;
                op[5] = *group;
                op[6..8].copy_from_slice(&target.to_le_bytes());
                op[8..0x0c].copy_from_slice(&mask_hi.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mask_lo.to_le_bytes());
            }
            Operation::BattleMoveTo {
                battle_slot,
                group,
                x,
                y,
                mask_hi,
                mask_lo,
            } => {
                let opcode: u32 = 0x95;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4] = *battle_slot;
                op[5] = *group;
                op[6..8].copy_from_slice(&x.to_le_bytes());
                op[8..0x0a].copy_from_slice(&y.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mask_hi.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&mask_lo.to_le_bytes());
            }
            Operation::BattleSetOrderMode {
                battle_slot,
                mode,
                modifier,
                group,
                mask_hi,
                mask_lo,
            } => {
                let opcode: u32 = 0x97;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4] = *battle_slot;
                op[5] = *mode;
                op[6] = *modifier as u8;
                op[7] = *group;
                op[8..0x0c].copy_from_slice(&mask_hi.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mask_lo.to_le_bytes());
            }
            Operation::BattleSetSail {
                battle_slot,
                value,
                group,
                mask_hi,
                mask_lo,
            } => {
                let opcode: u32 = 0x98;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4] = *battle_slot;
                op[5] = *value;
                op[6] = *group;
                op[8..0x0c].copy_from_slice(&mask_hi.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mask_lo.to_le_bytes());
            }
            Operation::BattleDisengage {
                battle_slot,
                group,
                mask_hi,
                mask_lo,
            } => {
                let opcode: u32 = 0x9a;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4] = *battle_slot;
                op[5] = *group;
                op[8..0x0c].copy_from_slice(&mask_hi.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mask_lo.to_le_bytes());
            }
            Operation::BattleFlee {
                battle_slot,
                flee,
                group,
                mask_hi,
                mask_lo,
            } => {
                let opcode: u32 = 0x9b;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4] = *battle_slot;
                op[5] = *flee as u8;
                op[6] = *group;
                op[8..0x0c].copy_from_slice(&mask_hi.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mask_lo.to_le_bytes());
            }
            Operation::HireAdministrator { merchant_index, town_index } | Operation::DismissAdministrator { merchant_index, town_index } => {
                let opcode: u32 = 0x5e;
                let mode: u32 = if matches!(self, Operation::HireAdministrator { .. }) { 0 } else { 1 };
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[4..6].copy_from_slice(&merchant_index.to_le_bytes());
                op[8..0x0a].copy_from_slice(&town_index.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&mode.to_le_bytes());
            }
            Operation::OfficeAutotradeLockChange {
                ware_id,
                town_index,
                merchant_index,
                lock,
            } => {
                let opcode: u32 = 0x66;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&(*ware_id as u32).to_le_bytes());
                // The executor passes +0x8 and +0xc straight to the office lookup
                // 0x5308a0, whose argument order is (merchant, town) - established by
                // the checkbox draw code at 0x5d9d59, which passes the player merchant
                // global (operations+0x924) first and the window's town second.
                op[0x08..0x0a].copy_from_slice(&merchant_index.to_le_bytes());
                op[0x0c..0x0e].copy_from_slice(&town_index.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&(*lock as u32).to_le_bytes());
            }
            Operation::SetTradeRouteActive { ship_index, active } => {
                let opcode: u32 = 0x68;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&ship_index.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&(*active as u32).to_le_bytes());
            }
            Operation::RemoveTradeRouteStop { stop_pool_index, ship_index } => {
                let opcode: u32 = 0x6a;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&stop_pool_index.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&0xffu32.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&ship_index.to_le_bytes());
            }
            Operation::InsertTradeRouteStop {
                after_stop_pool_index,
                town_index,
                ship_index,
            } => {
                let opcode: u32 = 0x6a;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&after_stop_pool_index.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&1u32.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&(*town_index as u32).to_le_bytes());
                op[0x10..0x14].copy_from_slice(&ship_index.to_le_bytes());
            }
            Operation::RenameShip { ship_index, name } => {
                let opcode: u32 = 0x2d;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x10].copy_from_slice(name);
                op[0x10..0x14].copy_from_slice(&ship_index.to_le_bytes());
            }
            Operation::AppendShipName { ship_index, name } => {
                let opcode: u32 = 0x2e;
                op[0..4].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x10].copy_from_slice(name);
                op[0x10..0x14].copy_from_slice(&ship_index.to_le_bytes());
            }
            Operation::SetGameSpeed {
                speed1_ms_per_tick,
                speed2_ms_per_tick,
                level,
                field_10,
            } => {
                let opcode: u32 = 0xc8;
                op[0x00..0x04].copy_from_slice(&opcode.to_le_bytes());
                op[0x04..0x08].copy_from_slice(&speed1_ms_per_tick.to_le_bytes());
                op[0x08..0x0c].copy_from_slice(&speed2_ms_per_tick.to_le_bytes());
                op[0x0c..0x10].copy_from_slice(&level.to_le_bytes());
                op[0x10..0x14].copy_from_slice(&field_10.to_le_bytes());
            }
        }
        op
    }
}

use crate::data::enums::WareId;

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
    ShipMoveWares {
        amount: i32,
        ship_index: u16,
        ware_id: WareId,
        merchant_index: u16,
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

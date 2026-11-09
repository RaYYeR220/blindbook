//! The resting book, persisted in the pool's storage cell.
//!
//! Stored as versioned JSON `{v, orders, in_flight}` under [`BOOK_KEY`] with
//! `storage::set` (keystore-encrypted, in the predecessor's cell). Resting
//! lots live in `orders`; a lot handed to settlement moves to `in_flight`
//! with its order's metadata until the verifier shows its nonce used (filled)
//! or its sub-batch can no longer settle (back to the book).

use std::collections::BTreeSet;

use blindbook_core::Side;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BOOK_KEY: &str = "book";
pub const BOOK_VERSION: u32 = 1;
/// Orders with at least one resting or in-flight lot.
pub const MAX_ORDERS: usize = 64;
/// Resting plus in-flight lots.
pub const MAX_LOTS: usize = 512;
/// Serialized size cap of the stored book.
pub const MAX_BOOK_BYTES: usize = 1 << 20;
/// Sub-batch failures an order may cause before it is dropped.
pub const MAX_STRIKES: u8 = 3;

mod u128_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// What an order is, independent of which of its lots remain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderMeta {
    pub id: String,
    pub market: String,
    pub owner: String,
    pub side: Side,
    pub min_lots: u16,
    pub seq: u64,
    /// Market assets and lot size the lots were validated against. An order
    /// only crosses while the market still has exactly these.
    pub base: String,
    pub quote: String,
    #[serde(with = "u128_str")]
    pub lot: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookLot {
    pub index: u16,
    #[serde(with = "u128_str")]
    pub limit_quote: u128,
    /// Base64 nonce of the signed payload.
    pub nonce: String,
    /// Intent deadline, Unix ns.
    pub deadline_ns: u64,
    /// The signed payload, passed to the verifier untouched.
    pub intent: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookOrder {
    pub meta: OrderMeta,
    #[serde(default)]
    pub strikes: u8,
    /// Resting lots, by index.
    pub lots: Vec<BookLot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InFlight {
    pub meta: OrderMeta,
    pub lot: BookLot,
    /// Deadline of the venue payload that settles it. After it passes the
    /// sub-batch can no longer execute.
    pub settle_by_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub v: u32,
    pub orders: Vec<BookOrder>,
    pub in_flight: Vec<InFlight>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub orders: usize,
    pub lots: usize,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            v: BOOK_VERSION,
            orders: vec![],
            in_flight: vec![],
        }
    }
}

impl Book {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let book: Book = serde_json::from_slice(bytes).map_err(|_| "book_corrupt")?;
        if book.v != BOOK_VERSION {
            return Err("book_corrupt");
        }
        Ok(book)
    }

    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        let bytes = serde_json::to_vec(self).map_err(|_| "book_corrupt")?;
        if bytes.len() > MAX_BOOK_BYTES {
            return Err("book_too_large");
        }
        Ok(bytes)
    }

    /// Distinct orders and total lots, resting and in flight.
    pub fn counts(&self) -> Counts {
        let mut ids: BTreeSet<&str> = self.orders.iter().map(|o| o.meta.id.as_str()).collect();
        ids.extend(self.in_flight.iter().map(|f| f.meta.id.as_str()));
        Counts {
            orders: ids.len(),
            lots: self.orders.iter().map(|o| o.lots.len()).sum::<usize>() + self.in_flight.len(),
        }
    }

    pub fn contains_order(&self, id: &str) -> bool {
        self.orders.iter().any(|o| o.meta.id == id) || self.in_flight.iter().any(|f| f.meta.id == id)
    }

    /// Every `(owner, nonce)` the book holds.
    pub fn nonces(&self) -> BTreeSet<(String, String)> {
        let resting = self
            .orders
            .iter()
            .flat_map(|o| o.lots.iter().map(move |l| (o.meta.owner.clone(), l.nonce.clone())));
        let flying = self
            .in_flight
            .iter()
            .map(|f| (f.meta.owner.clone(), f.lot.nonce.clone()));
        resting.chain(flying).collect()
    }

    pub fn has_in_flight(&self, order_id: &str) -> bool {
        self.in_flight.iter().any(|f| f.meta.id == order_id)
    }

    /// Put a lot back among its order's resting lots, recreating the order
    /// from the in-flight metadata when it has no resting lots left.
    pub fn restore(&mut self, f: InFlight) {
        match self.orders.iter_mut().find(|o| o.meta.id == f.meta.id) {
            Some(o) => {
                o.lots.push(f.lot);
                o.lots.sort_by_key(|l| l.index);
            }
            None => self.orders.push(BookOrder {
                meta: f.meta,
                strikes: 0,
                lots: vec![f.lot],
            }),
        }
    }

    /// Drop orders that can never cross again: no resting lots, or fewer than
    /// `min_lots` with nothing in flight that could come back.
    pub fn prune(&mut self) {
        let flying: BTreeSet<String> = self.in_flight.iter().map(|f| f.meta.id.clone()).collect();
        self.orders.retain(|o| {
            !o.lots.is_empty() && (o.lots.len() >= o.meta.min_lots as usize || flying.contains(&o.meta.id))
        });
        self.orders
            .sort_by(|a, b| (a.meta.seq, &a.meta.id).cmp(&(b.meta.seq, &b.meta.id)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(id: &str, min_lots: u16) -> OrderMeta {
        OrderMeta {
            id: id.into(),
            market: "m".into(),
            owner: "a.near".into(),
            side: Side::Sell,
            min_lots,
            seq: 1,
            base: "b".into(),
            quote: "q".into(),
            lot: 10,
        }
    }

    fn lot(index: u16) -> BookLot {
        BookLot {
            index,
            limit_quote: 5,
            nonce: format!("n{index}"),
            deadline_ns: 9,
            intent: json!({ "i": index }),
        }
    }

    #[test]
    fn round_trips_and_rejects_other_versions() {
        let mut b = Book::default();
        b.orders.push(BookOrder {
            meta: meta("1", 1),
            strikes: 0,
            lots: vec![lot(0), lot(1)],
        });
        let bytes = b.encode().unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["orders"][0]["lots"][0]["limit_quote"], "5");
        assert_eq!(Book::decode(&bytes).unwrap(), b);
        let mut v2 = v.clone();
        v2["v"] = json!(2);
        assert_eq!(Book::decode(v2.to_string().as_bytes()), Err("book_corrupt"));
        assert_eq!(Book::decode(b"garbage"), Err("book_corrupt"));
    }

    #[test]
    fn restore_merges_or_recreates() {
        let mut b = Book::default();
        b.orders.push(BookOrder {
            meta: meta("1", 1),
            strikes: 0,
            lots: vec![lot(1)],
        });
        b.restore(InFlight {
            meta: meta("1", 1),
            lot: lot(0),
            settle_by_ns: 0,
        });
        assert_eq!(b.orders[0].lots.iter().map(|l| l.index).collect::<Vec<_>>(), vec![0, 1]);
        b.restore(InFlight {
            meta: meta("2", 1),
            lot: lot(3),
            settle_by_ns: 0,
        });
        assert_eq!(b.orders.len(), 2);
        assert_eq!(b.counts(), Counts { orders: 2, lots: 3 });
    }

    #[test]
    fn prune_keeps_orders_that_can_still_cross() {
        let mut b = Book::default();
        b.orders.push(BookOrder {
            meta: meta("short", 2),
            strikes: 0,
            lots: vec![lot(0)],
        });
        b.orders.push(BookOrder {
            meta: meta("waiting", 2),
            strikes: 0,
            lots: vec![lot(0)],
        });
        b.orders.push(BookOrder {
            meta: meta("empty", 1),
            strikes: 0,
            lots: vec![],
        });
        b.orders.push(BookOrder {
            meta: meta("fine", 1),
            strikes: 0,
            lots: vec![lot(0)],
        });
        b.in_flight.push(InFlight {
            meta: meta("waiting", 2),
            lot: lot(1),
            settle_by_ns: 0,
        });
        b.prune();
        let ids: Vec<_> = b.orders.iter().map(|o| o.meta.id.as_str()).collect();
        assert_eq!(ids, vec!["fine", "waiting"]);
    }

    #[test]
    fn oversized_book_is_refused() {
        let mut b = Book::default();
        let big = json!({ "pad": "x".repeat(MAX_BOOK_BYTES) });
        b.orders.push(BookOrder {
            meta: meta("1", 1),
            strikes: 0,
            lots: vec![BookLot { intent: big, ..lot(0) }],
        });
        assert_eq!(b.encode(), Err("book_too_large"));
    }
}

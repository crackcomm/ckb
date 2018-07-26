use bigint::{H256, U256};
use core::block::IndexedBlock;
use core::header::{BlockNumber, Header, RawHeader, Seal};

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    // genesis data
    pub version: u32,
    pub parent_hash: H256,
    pub hash: H256,
    pub timestamp: u64,
    pub txs_commit: H256,
    pub difficulty: U256,
    pub number: BlockNumber,
    pub nonce: u64,
    pub mix_hash: H256,
    // other config
    pub initial_block_reward: Capacity,
}

impl Config {
    pub fn default() -> Self {
        Config {
            version: 0,
            parent_hash: H256::from(0),
            hash: H256::from("0x1d3c78fcf6a6c98b53aed1bfebe53d5d7a1a0b8dced33576e3806915ce51aa00"),
            timestamp: 0,
            txs_commit: H256::from(0),
            difficulty: U256::from(0),
            number: 0,
            nonce: 0,
            mix_hash: H256::from(0),
            initial_block_reward: 0,
        }
    }

    pub fn genesis_block(&self) -> IndexedBlock {
        let header = Header {
            raw: RawHeader {
                version: self.version,
                parent_hash: self.parent_hash,
                timestamp: self.timestamp,
                txs_commit: self.txs_commit,
                difficulty: self.difficulty,
                number: self.number,
            },
            seal: Seal {
                nonce: self.nonce,
                mix_hash: self.mix_hash,
            },
        };

        IndexedBlock {
            header: header.into(),
            transactions: vec![],
        }
    }
}

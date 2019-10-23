use crate::utils::since_from_absolute_epoch_number;
use crate::{Node, TXOSet, TXO};
use ckb_chain_spec::OUTPUT_INDEX_DAO;
use ckb_types::core::{Capacity, EpochNumberWithFraction, HeaderView};
use ckb_types::packed::WitnessArgs;
use ckb_types::{
    bytes::Bytes,
    core::{ScriptHashType, TransactionBuilder, TransactionView},
    packed::{CellDep, CellInput, CellOutput, OutPoint, Script},
    prelude::*,
};
use std::collections::HashSet;

// https://github.com/nervosnetwork/ckb-system-scripts/blob/1fd4cd3e2ab7e5ffbafce1f60119b95937b3c6eb/c/dao.c#L81
pub const LOCK_PERIOD_EPOCHES: u64 = 180;

pub struct DAOUser<'a> {
    node: &'a Node,
    always_utxo: TXOSet,
    deposit_utxo: TXOSet,
    prepare_utxo: TXOSet,
    withdraw_utxo: TXOSet,
}

impl<'a> DAOUser<'a> {
    pub fn new(node: &'a Node, always_utxo: TXOSet) -> Self {
        Self {
            node,
            always_utxo,
            deposit_utxo: Default::default(),
            prepare_utxo: Default::default(),
            withdraw_utxo: Default::default(),
        }
    }

    pub fn deposit(&mut self) -> TransactionView {
        assert!(!self.always_utxo.is_empty());
        let node = self.node;
        let inputs = self
            .always_utxo
            .iter()
            .map(|txo| CellInput::new(txo.out_point(), 0));
        let output_data = Bytes::from(&[0u8; 8][..]).pack();
        let outputs = {
            let always_outputs = self.always_utxo.boom(vec![]).outputs();
            // TRICK: When we change the always_outputs to deposit_outputs, the always_output's
            // capacity will be insufficient. So here use some always_outputs' capacity
            // as the "capacity filler".
            let always_len = always_outputs.len();
            let deposit_outputs = always_outputs.into_iter().skip(always_len / 2);
            deposit_outputs
                .map(|output: CellOutput| {
                    output
                        .as_builder()
                        .lock(node.always_success_script())
                        .type_(Some(self.dao_type_script()).pack())
                        .build_exact_capacity(Capacity::bytes(output_data.len()).unwrap())
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };
        let outputs_data = outputs
            .iter()
            .map(|_| output_data.clone())
            .collect::<Vec<_>>();
        let cell_deps = vec![node.always_success_cell_dep(), self.dao_cell_dep()];
        let tx = TransactionBuilder::default()
            .cell_deps(cell_deps)
            .inputs(inputs)
            .outputs(outputs)
            .outputs_data(outputs_data)
            .witness(Default::default())
            .build();
        self.deposit_utxo = TXOSet::from(&tx);
        tx
    }

    pub fn prepare(&mut self) -> TransactionView {
        assert!(!self.deposit_utxo.is_empty());
        let node = self.node;
        let deposit_utxo_headers = self.utxo_headers(&self.deposit_utxo);
        let inputs = deposit_utxo_headers
            .iter()
            .map(|(txo, _)| CellInput::new(txo.out_point(), 0));
        let outputs = deposit_utxo_headers.iter().map(|(txo, _)| {
            CellOutput::new_builder()
                .capacity(txo.capacity().pack())
                .lock(txo.lock())
                .type_(txo.type_())
                .build()
        });
        let outputs_data = deposit_utxo_headers.iter().map(|(_, header)| {
            let deposit_number = header.number();
            Bytes::from(deposit_number.to_le_bytes().to_vec()).pack()
        });
        let cell_deps = vec![node.always_success_cell_dep(), self.dao_cell_dep()];
        // NOTE: dao.c uses `deposit_header` to ensure the prepare_output.capacity == deposit_output.capacity
        let header_deps = deposit_utxo_headers
            .iter()
            .map(|(_, header)| header.hash())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let witnesses = deposit_utxo_headers
            .iter()
            .map(|(_, header)| {
                let index = header_deps
                    .iter()
                    .position(|hash| hash == &header.hash())
                    .unwrap() as u64;
                WitnessArgs::new_builder()
                    .input_type(Some(Bytes::from(index.to_le_bytes().to_vec())).pack())
                    .build()
                    .as_bytes()
                    .pack()
            })
            .collect::<Vec<_>>();
        let tx = TransactionBuilder::default()
            .inputs(inputs)
            .outputs(outputs)
            .cell_deps(cell_deps)
            .header_deps(header_deps)
            .witnesses(witnesses)
            .outputs_data(outputs_data)
            .build();
        self.prepare_utxo = TXOSet::from(&tx);
        tx
    }

    pub fn withdraw(&mut self) -> TransactionView {
        assert!(!self.prepare_utxo.is_empty());
        let node = self.node;
        let deposit_utxo_headers = self.utxo_headers(&self.deposit_utxo);
        let prepare_utxo_headers = self.utxo_headers(&self.prepare_utxo);
        let inputs = prepare_utxo_headers.iter().map(|(txo, _)| {
            let minimal_unlock_point = self.minimal_unlock_point(&txo.out_point());
            let since = since_from_absolute_epoch_number(minimal_unlock_point.full_value());
            CellInput::new(txo.out_point(), since)
        });
        let outputs = prepare_utxo_headers
            .iter()
            .map(|(txo, _)| {
                // NOTE: I just want to make these withdrawals have different withdraw_header, to make
                // they have different interest rate. Hence here I generate new block within the loop.
                let withdraw_hash = node.generate_block();

                let capacity = self
                    .node
                    .rpc_client()
                    .calculate_dao_maximum_withdraw(txo.out_point().into(), withdraw_hash.clone());
                CellOutput::new_builder()
                    .capacity(capacity.pack())
                    .lock(node.always_success_script())
                    .build()
            })
            .collect::<Vec<_>>();
        let cell_deps = vec![node.always_success_cell_dep(), self.dao_cell_dep()];
        let header_deps = deposit_utxo_headers
            .iter()
            .chain(prepare_utxo_headers.iter())
            .map(|(_, header)| header.hash())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let witnesses = deposit_utxo_headers
            .iter()
            .map(|(_, header)| {
                let index = header_deps
                    .iter()
                    .position(|hash| hash == &header.hash())
                    .unwrap() as u64;
                WitnessArgs::new_builder()
                    .input_type(Some(Bytes::from(index.to_le_bytes().to_vec())).pack())
                    .build()
                    .as_bytes()
                    .pack()
            })
            .collect::<Vec<_>>();
        let outputs_data = (0..outputs.len())
            .map(|_| Default::default())
            .collect::<Vec<_>>();
        let tx = TransactionBuilder::default()
            .inputs(inputs)
            .outputs(outputs)
            .cell_deps(cell_deps)
            .header_deps(header_deps)
            .witnesses(witnesses)
            .outputs_data(outputs_data)
            .build();
        self.withdraw_utxo = TXOSet::from(&tx);
        tx
    }

    pub fn dao_type_script(&self) -> Script {
        Script::new_builder()
            .code_hash(self.node.consensus().dao_type_hash().unwrap())
            .hash_type(ScriptHashType::Type.into())
            .build()
    }

    fn dao_cell_dep(&self) -> CellDep {
        let node = self.node;
        CellDep::new_builder()
            .out_point(OutPoint::new(
                node.consensus()
                    .genesis_block()
                    .transaction(0)
                    .unwrap()
                    .hash(),
                OUTPUT_INDEX_DAO as u32,
            ))
            .build()
    }

    fn utxo_headers(&self, utxo: &TXOSet) -> Vec<(TXO, HeaderView)> {
        utxo.iter()
            .map(|txo| {
                let tx_hash = txo.out_point().tx_hash();
                let header = self
                    .node
                    .rpc_client()
                    .get_transaction(tx_hash)
                    .and_then(|tx| tx.tx_status.block_hash)
                    .and_then(|block_hash| self.node.rpc_client().get_header(block_hash.pack()))
                    .map(Into::into)
                    .expect("get_deposit_header");
                (txo, header)
            })
            .collect()
    }

    fn minimal_unlock_point(&self, out_point: &OutPoint) -> EpochNumberWithFraction {
        let node = self.node;
        let tx_hash = out_point.tx_hash();
        let deposit_point = {
            let deposit_hash = node
                .rpc_client()
                .get_transaction(tx_hash)
                .unwrap()
                .tx_status
                .block_hash
                .unwrap();
            let deposit_header = node.rpc_client().get_header(deposit_hash.pack()).unwrap();
            EpochNumberWithFraction::from_full_value(deposit_header.inner.epoch.value())
        };
        EpochNumberWithFraction::new(
            deposit_point.number() + LOCK_PERIOD_EPOCHES,
            deposit_point.index(),
            deposit_point.length(),
        )
    }
}

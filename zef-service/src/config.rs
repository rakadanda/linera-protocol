// Copyright (c) Facebook, Inc. and its affiliates.
// Copyright (c) Zefchain Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::transport::NetworkProtocol;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::Path,
};
use zef_base::{account::AccountState, base_types::*, committee::Committee};
use zef_core::{client::AccountClientState, node::AuthorityClient};
use zef_storage::Storage;

pub trait Import: DeserializeOwned {
    fn read(path: &Path) -> Result<Self, std::io::Error> {
        let data = fs::read(path)?;
        Ok(serde_json::from_slice(data.as_slice())?)
    }
}

pub trait Export: Serialize {
    fn write(&self, path: &Path) -> Result<(), std::io::Error> {
        let file = OpenOptions::new().create(true).write(true).open(path)?;
        let mut writer = BufWriter::new(file);
        let data = serde_json::to_string_pretty(self).unwrap();
        writer.write_all(data.as_ref())?;
        writer.write_all(b"\n")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorityConfig {
    pub network_protocol: NetworkProtocol,
    pub name: AuthorityName,
    pub host: String,
    pub base_port: u32,
    pub num_shards: u32,
}

impl AuthorityConfig {
    pub fn print(&self) {
        let data = serde_json::to_string(self).unwrap();
        println!("{}", data);
    }
}

#[derive(Serialize, Deserialize)]
pub struct AuthorityServerConfig {
    pub authority: AuthorityConfig,
    pub key: KeyPair,
}

impl Import for AuthorityServerConfig {}
impl Export for AuthorityServerConfig {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommitteeConfig {
    pub authorities: Vec<AuthorityConfig>,
}

impl Import for CommitteeConfig {}
impl Export for CommitteeConfig {}

impl CommitteeConfig {
    pub fn into_committee(self) -> Committee {
        Committee::new(self.voting_rights())
    }

    fn voting_rights(&self) -> BTreeMap<AuthorityName, usize> {
        let mut map = BTreeMap::new();
        for authority in &self.authorities {
            map.insert(authority.name, 1);
        }
        map
    }
}

#[derive(Serialize, Deserialize)]
pub struct UserAccount {
    pub account_id: AccountId,
    pub key_pair: Option<KeyPair>,
    pub next_sequence_number: SequenceNumber,
}

impl UserAccount {
    pub fn new(account_id: AccountId) -> Self {
        Self {
            account_id,
            key_pair: None,
            next_sequence_number: SequenceNumber::new(),
        }
    }

    pub fn make_initial(account_id: AccountId) -> Self {
        let key_pair = KeyPair::generate();
        Self {
            account_id,
            key_pair: Some(key_pair),
            next_sequence_number: SequenceNumber::new(),
        }
    }
}

pub struct WalletState {
    accounts: BTreeMap<AccountId, UserAccount>,
}

impl WalletState {
    pub fn get(&self, account_id: &AccountId) -> Option<&UserAccount> {
        self.accounts.get(account_id)
    }

    pub fn get_or_insert(&mut self, account_id: AccountId) -> &UserAccount {
        self.accounts
            .entry(account_id.clone())
            .or_insert_with(|| UserAccount::new(account_id))
    }

    pub fn insert(&mut self, account: UserAccount) {
        self.accounts.insert(account.account_id.clone(), account);
    }

    pub fn num_accounts(&self) -> usize {
        self.accounts.len()
    }

    pub fn last_account(&mut self) -> Option<&UserAccount> {
        self.accounts.values().last()
    }

    pub fn accounts_mut(&mut self) -> impl Iterator<Item = &mut UserAccount> {
        self.accounts.values_mut()
    }

    pub async fn update_from_state<A, S>(&mut self, state: &mut AccountClientState<A, S>)
    where
        A: AuthorityClient + Send + Sync + 'static + Clone,
        S: Storage + Clone + 'static,
    {
        let account = self
            .accounts
            .entry(state.account_id().clone())
            .or_insert_with(|| UserAccount::new(state.account_id().clone()));
        account.key_pair = state.key_pair().await.map(|k| k.copy()).ok();
        account.next_sequence_number = state.next_sequence_number();
    }

    pub fn read_or_create(path: &Path) -> Result<Self, std::io::Error> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(path)?;
        let reader = BufReader::new(file);
        let stream = serde_json::Deserializer::from_reader(reader).into_iter();
        Ok(Self {
            accounts: stream
                .filter_map(Result::ok)
                .map(|account: UserAccount| (account.account_id.clone(), account))
                .collect(),
        })
    }

    pub fn write(&self, path: &Path) -> Result<(), std::io::Error> {
        let file = OpenOptions::new().write(true).open(path)?;
        let mut writer = BufWriter::new(file);
        for account in self.accounts.values() {
            serde_json::to_writer(&mut writer, account)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct GenesisConfig {
    pub committee: CommitteeConfig,
    pub accounts: Vec<(AccountId, AccountOwner, Balance)>,
}

impl Import for GenesisConfig {}
impl Export for GenesisConfig {}

impl GenesisConfig {
    pub fn new(committee: CommitteeConfig) -> Self {
        Self {
            committee,
            accounts: Vec::new(),
        }
    }

    pub async fn initialize_store<S>(&self, store: &mut S) -> Result<(), failure::Error>
    where
        S: Storage + Clone + 'static,
    {
        for (account_id, owner, balance) in &self.accounts {
            let account = AccountState::create(
                self.committee.clone().into_committee(),
                account_id.clone(),
                *owner,
                *balance,
            );
            store.write_account(account.clone()).await?;
        }
        Ok(())
    }
}

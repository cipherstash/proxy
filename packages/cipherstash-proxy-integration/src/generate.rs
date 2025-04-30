#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use cipherstash_client::config::EnvSource;
    use cipherstash_client::credentials::auto_refresh::AutoRefresh;
    use cipherstash_client::ejsonpath::Selector;
    use cipherstash_client::encryption::{
        Encrypted, EncryptedEntry, EncryptedSteVecTerm, JsonIndexer, JsonIndexerOptions, OreTerm,
        Plaintext, PlaintextTarget, QueryBuilder, ReferencedPendingPipeline,
    };
    use cipherstash_client::{
        encryption::{ScopedCipher, SteVec},
        zerokms::{encrypted_record, EncryptedRecord},
    };
    use cipherstash_client::{ConsoleConfig, CtsConfig, ZeroKMSConfig};
    use cipherstash_config::column::{Index, IndexType};
    use cipherstash_config::{ColumnConfig, ColumnMode, ColumnType};
    use cipherstash_proxy::Identifier;
    use rustls::unbuffered::EncodeError;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    use tracing::info;
    use uuid::Uuid;

    pub mod option_mp_base85 {
        use cipherstash_client::zerokms::encrypted_record::formats::mp_base85;
        use cipherstash_client::zerokms::EncryptedRecord;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S>(
            value: &Option<EncryptedRecord>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            match value {
                Some(record) => mp_base85::serialize(record, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<EncryptedRecord>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let result = Option::<EncryptedRecord>::deserialize(deserializer)?;
            Ok(result)
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct EqlEncrypted {
        #[serde(rename = "c", with = "option_mp_base85")]
        ciphertext: Option<EncryptedRecord>,
        #[serde(rename = "i")]
        identifier: Identifier,
        #[serde(rename = "v")]
        version: u16,

        #[serde(rename = "o")]
        ore_index: Option<Vec<String>>,
        #[serde(rename = "m")]
        match_index: Option<Vec<u16>>,
        #[serde(rename = "u")]
        unique_index: Option<String>,

        #[serde(rename = "s")]
        selector: Option<String>,

        #[serde(rename = "b")]
        blake3_index: Option<String>,

        #[serde(rename = "ocf")]
        ore_cclw_fixed_index: Option<String>,
        #[serde(rename = "ocv")]
        ore_cclw_var_index: Option<String>,

        #[serde(rename = "sv")]
        ste_vec_index: Option<Vec<EqlSteVecEncrypted>>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct EqlSteVecEncrypted {
        #[serde(rename = "c", with = "option_mp_base85")]
        ciphertext: Option<EncryptedRecord>,

        #[serde(rename = "s")]
        selector: Option<String>,
        #[serde(rename = "b")]
        blake3_index: Option<String>,
        #[serde(rename = "ocf")]
        ore_cclw_fixed_index: Option<String>,
        #[serde(rename = "ocv")]
        ore_cclw_var_index: Option<String>,
    }

    impl EqlEncrypted {
        pub fn ste_vec(ste_vec_index: Vec<EqlSteVecEncrypted>) -> Self {
            Self {
                ste_vec_index: Some(ste_vec_index),
                ciphertext: None,
                identifier: Identifier {
                    table: "blah".to_string(),
                    column: "vtha".to_string(),
                },
                version: 1,
                ore_index: None,
                match_index: None,
                unique_index: None,
                selector: None,
                ore_cclw_fixed_index: None,
                ore_cclw_var_index: None,
                blake3_index: None,
            }
        }
    }
    impl EqlSteVecEncrypted {
        pub fn ste_vec_element(selector: String, record: EncryptedRecord) -> Self {
            Self {
                ciphertext: Some(record),
                selector: Some(selector),
                ore_cclw_fixed_index: None,
                ore_cclw_var_index: None,
                blake3_index: None,
            }
        }
    }

    #[tokio::test]
    async fn generate_ste_vec() {
        trace();

        // clear().await;
        // let client = connect_with_tls(PROXY).await;

        let console_config = ConsoleConfig::builder().with_env().build().unwrap();
        let cts_config = CtsConfig::builder().with_env().build().unwrap();
        let zerokms_config = ZeroKMSConfig::builder()
            .add_source(EnvSource::default())
            .console_config(&console_config)
            .cts_config(&cts_config)
            .build_with_client_key()
            .unwrap();
        let zerokms_client = zerokms_config
            .create_client_with_credentials(AutoRefresh::new(zerokms_config.credentials()));

        let dataset_id = Uuid::parse_str("295504329cb045c398dc464c52a287a1").unwrap();

        let cipher = Arc::new(
            ScopedCipher::init(Arc::new(zerokms_client), Some(dataset_id))
                .await
                .unwrap(),
        );

        let prefix = "prefix".to_string();

        let column_config = ColumnConfig::build("column_name".to_string())
            .casts_as(ColumnType::JsonB)
            .add_index(Index::new(IndexType::SteVec {
                prefix: prefix.to_owned(),
            }));

        // let mut value =
        //     serde_json::from_str::<serde_json::Value>("{\"hello\": \"one\", \"n\": 10}").unwrap();

        // let mut value =
        //     serde_json::from_str::<serde_json::Value>("{\"hello\": \"two\", \"n\": 20}").unwrap();

        let mut value =
            serde_json::from_str::<serde_json::Value>("{\"hello\": \"two\", \"n\": 30}").unwrap();

        // let mut value =
        //     serde_json::from_str::<serde_json::Value>("{\"hello\": \"world\", \"n\": 42}").unwrap();

        // let mut value =
        //     serde_json::from_str::<serde_json::Value>("{\"hello\": \"world\", \"n\": 42}").unwrap();

        // let mut value =
        //     serde_json::from_str::<serde_json::Value>("{\"blah\": { \"vtha\": 42 }}").unwrap();

        let plaintext = Plaintext::JsonB(Some(value));

        let idx = 0;

        let mut pipeline = ReferencedPendingPipeline::new(cipher.clone());
        let encryptable = PlaintextTarget::new(plaintext, column_config);
        pipeline
            .add_with_ref::<PlaintextTarget>(encryptable, idx)
            .unwrap();

        let mut encrypteds = vec![];

        let mut result = pipeline.encrypt(None).await.unwrap();
        if let Some(Encrypted::SteVec(ste_vec)) = result.remove(idx) {
            for entry in ste_vec {
                let selector = hex::encode(entry.0 .0);
                let term = entry.1;
                let record = entry.2;

                let mut e = EqlSteVecEncrypted::ste_vec_element(selector, record);

                match term {
                    EncryptedSteVecTerm::Mac(items) => {
                        e.blake3_index = Some(hex::encode(&items));
                    }
                    EncryptedSteVecTerm::OreFixed(o) => {
                        e.ore_cclw_fixed_index = Some(hex::encode(o.bytes));
                    }
                    EncryptedSteVecTerm::OreVariable(o) => {
                        e.ore_cclw_var_index = Some(hex::encode(o.bytes));
                    }
                }

                encrypteds.push(e);
            }
            // info!("{:?}" = encrypteds);
        }

        info!("---------------------------------------------");

        let e = EqlEncrypted::ste_vec(encrypteds);
        info!("{:?}" = ?e);

        let json = serde_json::to_value(e).unwrap();
        info!("{}", json);

        let indexer = JsonIndexer::new(JsonIndexerOptions { prefix });

        info!("---------------------------------------------");

        // Path
        // let path: String = "$.blah.vtha".to_string();
        // let selector = Selector::parse(&path).unwrap();
        // let selector = indexer.generate_selector(selector, cipher.index_key());
        // let selector = hex::encode(selector.0);
        // info!("{}", selector);

        // Comparison
        let n = 30;
        let term = OreTerm::Number(n);

        let term = indexer.generate_term(term, cipher.index_key()).unwrap();

        match term {
            EncryptedSteVecTerm::Mac(items) => todo!(),
            EncryptedSteVecTerm::OreFixed(ore_cllw8_v1) => {
                let term = hex::encode(ore_cllw8_v1.bytes);
                info!("{n}: {term}");
            }
            EncryptedSteVecTerm::OreVariable(ore_cllw8_variable_v1) => todo!(),
        }

        // if let Some(ste_vec_index) = e.ste_vec_index {
        //     for e in ste_vec_index {
        //         info!("{}", e);
        //         if let Some(ct) = e.ciphertext {
        //             let decrypted = cipher.decrypt(encrypted).await?;
        //             info!("{}", decrypted);
        //         }
        //     }
        // }
    }
}

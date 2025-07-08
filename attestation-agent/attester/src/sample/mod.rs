// Copyright (c) 2022 Alibaba Cloud
//
// SPDX-License-Identifier: Apache-2.0
//

use super::{Attester, TeeEvidence};
use anyhow::*;
use base64::Engine;
use log::warn;
use serde::{Deserialize, Serialize};
use crate::tpm_utils::{TpmQuote, generate_rsa_ak, get_quote, extend_pcr, detect_tpm_device};
use std::result::Result as StdResult;

// Sample attester is always supported
pub fn detect_platform() -> bool {
    true
}

/// Evidence structure for the Sample Attester.
///
/// - `svn`: Security version number (dummy for sample)
/// - `report_data`: Base64-encoded report data
/// - `tpm_quote`: Optional TPM quote if a TPM device is present
#[derive(Serialize, Deserialize, Debug)]
pub struct Evidence {
    pub svn: String,
    pub report_data: String,
    pub tpm_quote: Option<TpmQuote>,
}

#[derive(Debug, Default)]
pub struct SampleAttester {}

#[async_trait::async_trait]
impl Attester for SampleAttester {
    /// Get evidence for the sample attester.
    ///
    /// If a TPM device is present (/dev/tpm0 or /dev/tpm1),
    /// includes a TPM quote in the evidence. Otherwise, tpm_quote is None.
    async fn get_evidence(&self, report_data: Vec<u8>) -> Result<TeeEvidence> {
        // Check for TPM device
        let tpm_device = detect_tpm_device();

        let tpm_quote = if let Some(dev) = tpm_device {
            log::info!("Using TPM device: {}", dev);
            // Set TCTI env var for tss-esapi
            std::env::set_var("TCTI", format!("device:{}", dev));
            // Limit report_data size to 64 bytes for TPM
            let data = if report_data.len() > 64 {
                &report_data[..64]
            } else {
                &report_data
            };
            match generate_rsa_ak().and_then(|ak| get_quote(ak, data, "SHA256")) {
                StdResult::Ok(q) => Some(q),
                StdResult::Err(e) => {
                    log::warn!("Failed to get TPM quote: {e}");
                    None
                }
            }
        } else {
            None
        };

        let evidence = Evidence {
            svn: "1".to_string(),
            report_data: base64::engine::general_purpose::STANDARD.encode(&report_data),
            tpm_quote,
        };
        serde_json::to_value(&evidence).context("Serialize sample evidence failed")
    }

    /// Extend runtime measurement for the sample attester.
    ///
    /// If a TPM device is present, extends the PCR using the TPM.
    /// Otherwise, logs a warning and does nothing.
    async fn extend_runtime_measurement(
        &self,
        event_digest: Vec<u8>,
        register_index: u64,
    ) -> Result<()> {
        let tpm_device = detect_tpm_device();
        if let Some(dev) = tpm_device {
            log::info!("Using TPM device: {}", dev);
            std::env::set_var("TCTI", format!("device:{}", dev));
            extend_pcr(event_digest, register_index)
                .map_err(|e| anyhow!("Failed to extend PCR: {e}"))
        } else {
            warn!("No TPM device found, cannot extend runtime measurement.");
            Ok(())
        }
    }

    async fn get_runtime_measurement(&self, _pcr_index: u64) -> Result<Vec<u8>> {
        Ok(vec![])
    }

    /// Bind init data for the sample attester.
    ///
    /// If a TPM device is present, extends PCR 8 with the init_data_digest.
    /// Otherwise, returns Unsupported.
    async fn bind_init_data(&self, init_data_digest: &[u8]) -> Result<super::InitDataResult> {
        let tpm_device = detect_tpm_device();
        if let Some(dev) = tpm_device {
            log::info!("Using TPM device: {}", dev);
            std::env::set_var("TCTI", format!("device:{}", dev));
            // PCR 8 is used for init data
            extend_pcr(init_data_digest.to_vec(), 8)
                .map_err(|e| anyhow!("Failed to extend PCR for init data: {e}"))?;
            Ok(super::InitDataResult::Ok)
        } else {
            Ok(super::InitDataResult::Unsupported)
        }
    }
}

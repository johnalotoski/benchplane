// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::Experiment;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("experiment YAML is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("could not parse experiment YAML: {0}")]
    Yaml(#[from] serde_saphyr::Error),
}

pub fn parse_experiment(bytes: &[u8]) -> Result<Experiment, ParseError> {
    let source = std::str::from_utf8(bytes)?;
    Ok(serde_saphyr::from_str(source)?)
}

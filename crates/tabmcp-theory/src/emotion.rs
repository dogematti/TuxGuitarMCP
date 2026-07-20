//! Emotion targets: a vocabulary that maps feeling words onto the tension
//! scale, so "calm -> uneasy -> aggressive -> victorious" becomes a curve
//! the AI Ear can check the music against.

pub const EMOTIONS: &[(&str, f64)] = &[
    ("calm", 0.15),
    ("dreamy", 0.20),
    ("mournful", 0.25),
    ("hopeless", 0.30),
    ("brooding", 0.35),
    ("uneasy", 0.45),
    ("menacing", 0.55),
    ("tense", 0.65),
    ("driving", 0.70),
    ("epic", 0.75),
    ("aggressive", 0.80),
    ("triumphant", 0.85),
    ("victorious", 0.90),
    ("furious", 0.95),
];

/// Parse "calm, uneasy, aggressive, victorious" into a tension curve plus
/// the labels. Err lists the vocabulary when a word is unknown.
pub fn curve(spec: &str) -> Result<(Vec<f64>, Vec<String>), String> {
    let mut values = Vec::new();
    let mut labels = Vec::new();
    for raw in spec.split([',', '>']) {
        let word = raw.trim().trim_start_matches('-').trim().to_ascii_lowercase();
        if word.is_empty() {
            continue;
        }
        match EMOTIONS.iter().find(|(name, _)| *name == word) {
            Some((name, value)) => {
                values.push(*value);
                labels.push(name.to_string());
            }
            None => {
                return Err(format!(
                    "unknown emotion '{word}'; vocabulary: {}",
                    EMOTIONS
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
    if values.is_empty() {
        return Err("empty emotion journey".into());
    }
    Ok((values, labels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journey_parses_to_a_rising_curve() {
        let (values, labels) = curve("calm, uneasy, aggressive, victorious").expect("parses");
        assert_eq!(labels, vec!["calm", "uneasy", "aggressive", "victorious"]);
        assert!(values.windows(2).all(|w| w[0] < w[1]));
        assert!(curve("calm, transcendent").is_err());
        // Arrow-style specs also parse.
        assert!(curve("calm > furious").is_ok());
    }
}

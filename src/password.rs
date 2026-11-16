use rand::seq::SliceRandom;
use rand::Rng;

use crate::config::GeneratorConfig;

const AMBIGUOUS: &str = "il1LIo0O";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?/";

// ponytail: small embedded wordlist; swap for the EFF long list if passphrases matter.
const WORDS: &[&str] = &[
    "amber", "anchor", "apple", "arrow", "atlas", "autumn", "bacon", "badge", "bamboo", "basil",
    "beacon", "berry", "bison", "blade", "bloom", "bolt", "breeze", "bridge", "bronze", "brook",
    "cabin", "cactus", "camel", "canyon", "cargo", "cedar", "chalk", "cherry", "cider", "cobalt",
    "comet", "copper", "coral", "cosmos", "cotton", "crane", "crater", "crimson", "crystal",
    "daisy", "dawn", "delta", "denim", "desert", "dolphin", "dragon", "dune", "eagle", "echo",
    "ember", "emerald", "falcon", "fern", "fiber", "field", "flame", "flint", "forest", "fossil",
    "fox", "frost", "galaxy", "garden", "garnet", "ginger", "glacier", "glade", "granite", "grape",
    "harbor", "hazel", "heron", "hickory", "hollow", "honey", "horizon", "indigo", "iris", "ivory",
    "jade", "jasmine", "jasper", "jungle", "juniper", "kettle", "kiwi", "lagoon", "lantern",
    "lark", "lava", "lemon", "lilac", "linen", "lunar", "maple", "marble", "meadow", "mesa",
    "meteor", "mint", "moss", "nectar", "nickel", "north", "oak", "oasis", "ocean", "olive",
    "onyx", "opal", "orbit", "otter", "pebble", "pepper", "pine", "plum", "prairie", "quartz",
    "quill", "rain", "raven", "reef", "ridge", "river", "robin", "rose", "ruby", "saffron", "sage",
    "sand", "scarlet", "shadow", "silver", "sky", "slate", "snow", "solar", "sparrow", "spruce",
    "star", "steel", "stone", "storm", "summit", "sunset", "tiger", "timber", "topaz", "trail",
    "tulip", "tundra", "umber", "valley", "velvet", "violet", "walnut", "willow", "wind", "wolf",
    "zephyr",
];

pub fn generate(options: &GeneratorConfig) -> Result<String, String> {
    if options.passphrase {
        return Ok(generate_passphrase(options));
    }

    let mut pools: Vec<&str> = Vec::new();
    if options.uppercase {
        pools.push("ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }
    if options.lowercase {
        pools.push("abcdefghijklmnopqrstuvwxyz");
    }
    if options.numbers {
        pools.push("0123456789");
    }
    if options.symbols {
        pools.push(SYMBOLS);
    }
    if pools.is_empty() {
        return Err("Select at least one character set".into());
    }

    let filtered: Vec<String> = pools
        .iter()
        .map(|pool| {
            pool.chars()
                .filter(|c| !options.exclude_ambiguous || !AMBIGUOUS.contains(*c))
                .collect::<String>()
        })
        .filter(|pool| !pool.is_empty())
        .collect();
    if filtered.is_empty() {
        return Err("Every selected character set only contains ambiguous characters".into());
    }

    let length = options.length.clamp(filtered.len(), 256);
    let mut rng = rand::thread_rng();
    let mut chars: Vec<char> = Vec::with_capacity(length);

    // Guarantee one character from each selected set, then fill randomly.
    for pool in &filtered {
        let pool: Vec<char> = pool.chars().collect();
        chars.push(pool[rng.gen_range(0..pool.len())]);
    }
    let all: Vec<char> = filtered.iter().flat_map(|p| p.chars()).collect();
    while chars.len() < length {
        chars.push(all[rng.gen_range(0..all.len())]);
    }
    chars.shuffle(&mut rng);

    Ok(chars.into_iter().collect())
}

pub fn generate_passphrase(options: &GeneratorConfig) -> String {
    let words = options.words.clamp(3, 24);
    let mut rng = rand::thread_rng();
    let separator = if options.separator.is_empty() {
        "-"
    } else {
        options.separator.as_str()
    };
    (0..words)
        .map(|_| WORDS[rng.gen_range(0..WORDS.len())])
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_length_and_classes() {
        let options = GeneratorConfig {
            length: 32,
            ..GeneratorConfig::default()
        };
        let password = generate(&options).unwrap();
        assert_eq!(password.chars().count(), 32);
        assert!(password.chars().any(|c| c.is_ascii_uppercase()));
        assert!(password.chars().any(|c| c.is_ascii_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
        assert!(password.chars().any(|c| SYMBOLS.contains(c)));
    }

    #[test]
    fn excludes_ambiguous() {
        let options = GeneratorConfig {
            length: 64,
            exclude_ambiguous: true,
            ..GeneratorConfig::default()
        };
        let password = generate(&options).unwrap();
        assert!(!password.chars().any(|c| AMBIGUOUS.contains(c)));
    }

    #[test]
    fn passphrase_has_requested_words() {
        let options = GeneratorConfig {
            passphrase: true,
            words: 6,
            ..GeneratorConfig::default()
        };
        let phrase = generate(&options).unwrap();
        assert_eq!(phrase.split('-').count(), 6);
    }

    #[test]
    fn empty_charset_is_rejected() {
        let options = GeneratorConfig {
            uppercase: false,
            lowercase: false,
            numbers: false,
            symbols: false,
            ..GeneratorConfig::default()
        };
        assert!(generate(&options).is_err());
    }
}

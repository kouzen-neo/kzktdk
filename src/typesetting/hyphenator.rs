use std::collections::HashSet;

/// Intelligent hyphenation engine for comic/manga typesetting.
/// Breaks long words into valid phonetic syllables for Indonesian and English/Latin text
/// to enable balanced line wrapping and clean bubble fitting with trailing hyphens ('-').
pub struct Hyphenator;

impl Hyphenator {
    const VOWELS: [char; 10] = ['a', 'i', 'u', 'e', 'o', 'A', 'I', 'U', 'E', 'O'];
    const DIPHTHONGS_ID: [&'static str; 4] = ["ai", "au", "oi", "ei"];
    const DIGRAPHS_ID: [&'static str; 4] = ["ng", "ny", "sy", "kh"];

    const EN_PREFIXES: [&'static str; 16] = [
        "trans", "inter", "super", "under", "over", "anti", "auto", "pre", "pro", "sub", "mis",
        "non", "dis", "con", "com", "per",
    ];

    const EN_SUFFIXES: [&'static str; 15] = [
        "tional", "ssion", "sion", "tion", "ment", "able", "ible", "ness", "less", "ship", "hood",
        "fully", "ing", "ous", "ful",
    ];

    fn is_vowel(c: char) -> bool {
        Self::VOWELS.contains(&c)
    }

    fn is_consonant(c: char) -> bool {
        c.is_alphabetic() && !Self::is_vowel(c)
    }

    /// Extracts (prefix_punct, core_word, suffix_punct) from a token.
    fn extract_word_and_punctuation(token: &str) -> (&str, &str, &str) {
        let chars: Vec<(usize, char)> = token.char_indices().collect();
        if chars.is_empty() {
            return ("", token, "");
        }

        let mut start_idx = chars.len();
        for (i, &(_, c)) in chars.iter().enumerate() {
            if c.is_alphanumeric() {
                start_idx = i;
                break;
            }
        }

        let mut end_idx = 0;
        for (i, &(_, c)) in chars.iter().enumerate().rev() {
            if c.is_alphanumeric() {
                end_idx = i + 1;
                break;
            }
        }

        if start_idx >= end_idx {
            return ("", token, "");
        }

        let start_byte = chars[start_idx].0;
        let end_byte = if end_idx < chars.len() {
            chars[end_idx].0
        } else {
            token.len()
        };

        (
            &token[..start_byte],
            &token[start_byte..end_byte],
            &token[end_byte..],
        )
    }

    /// Splits a word into syllables according to Indonesian phonotactic & EYD rules.
    pub fn hyphenate_indonesian(word: &str) -> Vec<String> {
        let (prefix_punct, core_word, suffix_punct) = Self::extract_word_and_punctuation(word);
        if core_word.chars().count() < 10 {
            return vec![word.to_string()];
        }

        let clean: Vec<char> = core_word.to_lowercase().chars().collect();
        let len = clean.len();
        let mut breaks = Vec::new();
        let mut i = 0;

        let diphthongs: HashSet<&str> = Self::DIPHTHONGS_ID.iter().copied().collect();
        let digraphs: HashSet<&str> = Self::DIGRAPHS_ID.iter().copied().collect();

        while i < len {
            if i + 1 < len {
                let c1 = clean[i];
                let c2 = clean[i + 1];

                // Pattern 1: V-V -> split between vowels
                if Self::is_vowel(c1) && Self::is_vowel(c2) {
                    let pair: String = [c1, c2].iter().collect();
                    let is_diphthong_open = diphthongs.contains(pair.as_str())
                        && (i + 2 >= len || !Self::is_consonant(clean[i + 2]));
                    if !is_diphthong_open {
                        breaks.push(i + 1);
                    }
                    i += 1;
                    continue;
                }

                // Pattern 2: V-K...
                if Self::is_vowel(c1) && Self::is_consonant(c2) && i + 2 < len {
                    let c3 = clean[i + 2];
                    let pair23: String = [c2, c3].iter().collect();

                    if digraphs.contains(pair23.as_str()) {
                        if i + 3 < len && Self::is_vowel(clean[i + 3]) {
                            // V - Digraph - V (e.g. ba-nyak, ta-ngan, ke-ba-nya-kan)
                            breaks.push(i + 1);
                            i += 3;
                            continue;
                        } else if i + 3 < len && Self::is_consonant(clean[i + 3]) {
                            // V - Digraph - K - V (e.g. ang-klung)
                            breaks.push(i + 3);
                            i += 3;
                            continue;
                        }
                    }

                    // Pattern 2a: V - K - V (e.g. ba-tu, ma-kan)
                    if Self::is_vowel(c3) {
                        breaks.push(i + 1);
                        i += 2;
                        continue;
                    }

                    // Pattern 3: V - K1 - K2 - ...
                    if Self::is_consonant(c3) {
                        if i + 3 < len {
                            let c4 = clean[i + 3];
                            if Self::is_vowel(c4) {
                                // Pattern 3a: V - K1 - K2 - V (e.g. am-bil, ban-tu)
                                breaks.push(i + 2);
                                i += 3;
                                continue;
                            } else if Self::is_consonant(c4) {
                                // Pattern 4: V - K1 - K2 - K3 - V (e.g. ben-trok)
                                breaks.push(i + 2);
                                i += 3;
                                continue;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            i += 1;
        }

        // Filter valid break indices: min 3 chars at start and min 3 chars at end
        let core_len = clean.len();
        breaks.sort_unstable();
        breaks.dedup();
        let valid_breaks: Vec<usize> = breaks
            .into_iter()
            .filter(|&b| b >= 3 && b <= core_len.saturating_sub(3))
            .collect();

        if valid_breaks.is_empty() {
            return vec![word.to_string()];
        }

        let chars_orig: Vec<char> = core_word.chars().collect();
        let mut syllables = Vec::new();
        let mut last_idx = 0;
        for &b in &valid_breaks {
            let s: String = chars_orig[last_idx..b].iter().collect();
            syllables.push(s);
            last_idx = b;
        }
        let last_part: String = chars_orig[last_idx..].iter().collect();
        syllables.push(last_part);

        if !syllables.is_empty() {
            syllables[0] = format!("{}{}", prefix_punct, syllables[0]);
            let last_idx = syllables.len() - 1;
            syllables[last_idx] = format!("{}{}", syllables[last_idx], suffix_punct);
        }

        syllables
    }

    /// Splits a word into syllables for English/Latin fallback.
    pub fn hyphenate_english(word: &str) -> Vec<String> {
        let (prefix_punct, core_word, suffix_punct) = Self::extract_word_and_punctuation(word);
        if core_word.chars().count() < 10 {
            return vec![word.to_string()];
        }

        let clean_lower = core_word.to_lowercase();
        let clean: Vec<char> = clean_lower.chars().collect();
        let len = clean.len();
        let mut breaks = Vec::new();

        // 1. Check known prefixes
        for &pref in &Self::EN_PREFIXES {
            if clean_lower.starts_with(pref) && clean_lower.len().saturating_sub(pref.len()) >= 3 {
                breaks.push(pref.chars().count());
                break;
            }
        }

        // 2. Check known suffixes
        for &suff in &Self::EN_SUFFIXES {
            if clean_lower.ends_with(suff) && clean_lower.len().saturating_sub(suff.len()) >= 3 {
                breaks.push(clean_lower.chars().count() - suff.chars().count());
                break;
            }
        }

        // 3. Fallback consonant-vowel syllable boundary detection
        let mut i = 1;
        while i + 2 < len {
            let c1 = clean[i];
            let c2 = clean[i + 1];

            // Double consonants (e.g. let-ter, hap-py)
            if Self::is_consonant(c1)
                && c1 == c2
                && Self::is_vowel(clean[i - 1])
                && i + 2 < len
                && Self::is_vowel(clean[i + 2])
            {
                breaks.push(i + 1);
                i += 2;
                continue;
            }

            // V-C-C-V pattern
            if Self::is_vowel(clean[i - 1])
                && Self::is_consonant(c1)
                && Self::is_consonant(c2)
                && i + 2 < len
                && Self::is_vowel(clean[i + 2])
            {
                let pair: String = [c1, c2].iter().collect();
                if !["th", "sh", "ch", "ph", "wh", "ck"].contains(&pair.as_str()) {
                    breaks.push(i + 1);
                    i += 2;
                    continue;
                }
            }

            // V-C-V pattern
            if Self::is_vowel(clean[i - 1])
                && Self::is_consonant(c1)
                && Self::is_vowel(c2)
                && i >= 2
                && len.saturating_sub(i) >= 3
            {
                breaks.push(i);
                i += 2;
                continue;
            }
            i += 1;
        }

        breaks.sort_unstable();
        breaks.dedup();
        let valid_breaks: Vec<usize> = breaks
            .into_iter()
            .filter(|&b| b >= 3 && b <= len.saturating_sub(3))
            .collect();

        if valid_breaks.is_empty() {
            return vec![word.to_string()];
        }

        let chars_orig: Vec<char> = core_word.chars().collect();
        let mut syllables = Vec::new();
        let mut last_idx = 0;
        for &b in &valid_breaks {
            let s: String = chars_orig[last_idx..b].iter().collect();
            syllables.push(s);
            last_idx = b;
        }
        let last_part: String = chars_orig[last_idx..].iter().collect();
        syllables.push(last_part);

        if !syllables.is_empty() {
            syllables[0] = format!("{}{}", prefix_punct, syllables[0]);
            let last_idx = syllables.len() - 1;
            syllables[last_idx] = format!("{}{}", syllables[last_idx], suffix_punct);
        }

        syllables
    }

    /// Splits a word into syllables based on target language.
    pub fn hyphenate(word: &str, target_language: Option<&str>) -> Vec<String> {
        let lang = target_language.unwrap_or("id").to_lowercase();
        if lang.contains("indo") || lang == "id" {
            Self::hyphenate_indonesian(word)
        } else if lang.contains("eng") || lang == "en" {
            Self::hyphenate_english(word)
        } else {
            Self::hyphenate_indonesian(word)
        }
    }

    /// Returns all candidate prefix substrings with a trailing hyphen that can fit on a line,
    /// ordered from longest prefix to shortest.
    pub fn get_hyphenation_candidates(
        word: &str,
        target_language: Option<&str>,
    ) -> Vec<(String, String)> {
        let mut candidates = Vec::new();
        let (prefix_punct, core_word, suffix_punct) = Self::extract_word_and_punctuation(word);

        // If the token contains an embedded hyphen (e.g. "NICO-HYAN", "KATA-KATA"),
        // splitting at that hyphen is the most natural break point in typography.
        if let Some(hyphen_idx) = core_word.find('-')
            && hyphen_idx > 0
            && hyphen_idx + 1 < core_word.len()
        {
            let prefix = format!("{}{}-", prefix_punct, &core_word[..hyphen_idx]);
            let suffix = format!("{}{}", &core_word[hyphen_idx + 1..], suffix_punct);
            candidates.push((prefix, suffix));
        }

        let syllables = Self::hyphenate(word, target_language);
        if syllables.len() > 1 {
            let total = syllables.len();
            for count in (1..total).rev() {
                let prefix = syllables[..count].join("") + "-";
                let suffix = syllables[count..].join("");
                candidates.push((prefix, suffix));
            }
        }

        candidates
    }
}

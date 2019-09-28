// Copyright 2018-2019 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::collections::HashMap;

use lazy_static::lazy_static;
use log::debug;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

type PreprocFn = dyn Fn(&str) -> String;

/// A list of preprocessors that normalize text without removing anything
/// substantial. These operate on one line at a time.
pub const PREPROC_NORMALIZE: [&PreprocFn; 6] = [
    &normalize_unicode,
    &remove_junk,
    &blackbox_urls,
    &normalize_horizontal_whitespace,
    &normalize_punctuation,
    &trim_line,
];

/// A list of preprocessors that more aggressively normalize/mangle text
/// to make for friendlier matching. May remove statements and lines, and
/// more heavily normalize punctuation.
pub const PREPROC_AGGRESSIVE: [&PreprocFn; 8] = [
    &remove_common_tokens,
    &normalize_vertical_whitespace,
    &remove_punctuation,
    &lowercaseify,
    &remove_title_line,
    &remove_copyright_statements,
    &collapse_whitespace,
    &final_trim,
];

pub fn apply_normalizers(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut out = line.to_owned();
        for preproc in &PREPROC_NORMALIZE {
            out = preproc(&out);
        }
        lines.push(out);
    }
    debug!("Normalized to:\n{:?}\n---", lines);
    lines
}

pub fn apply_aggressive(text: &str) -> String {
    let mut out = text.to_owned();
    for preproc in &PREPROC_AGGRESSIVE {
        out = preproc(&out);
    }
    debug!("Aggressively normalized to:\n{}\n---", out);
    out
}

// Line-by-line normalizers

fn normalize_unicode(input: &str) -> String {
    input.nfc().collect::<String>()
}

fn remove_junk(input: &str) -> String {
    lazy_static! {
        static ref RX: Regex = Regex::new(r"[^\w\s\pP]+").unwrap();
    }
    RX.replace_all(input, "").into()
}

fn blackbox_urls(input: &str) -> String {
    lazy_static! {
        static ref RX: Regex = Regex::new(r"https?://\S+").unwrap();
    }
    RX.replace_all(input, "http://blackboxed/url").into()
}

fn normalize_horizontal_whitespace(input: &str) -> String {
    lazy_static! {
        // including slashes here as well
        static ref RX: Regex = Regex::new(r"(?x)[ \t\p{Zs} \\ / \| \x2044 ]+").unwrap();
    }
    RX.replace_all(input, " ").into()
}

fn normalize_punctuation(input: &str) -> String {
    lazy_static! {
        static ref RX_QUOTES: Regex = Regex::new(r#"["'\p{Pi}\p{Pf}]+"#).unwrap();
        static ref RX_DASH: Regex = Regex::new(r"\p{Pd}+").unwrap();
        static ref RX_OPEN: Regex = Regex::new(r"\p{Ps}+").unwrap();
        static ref RX_CLOSE: Regex = Regex::new(r"\p{Pe}+").unwrap();
        static ref RX_UNDER: Regex = Regex::new(r"\p{Pc}+").unwrap();
        static ref RX_COPY: Regex = Regex::new(r"[©Ⓒⓒ]").unwrap();
    }
    let out = Cow::Borrowed(input);
    let out = RX_QUOTES.replace_all(&out, "'");
    let out = RX_DASH.replace_all(&out, "-");
    let out = RX_OPEN.replace_all(&out, "(");
    let out = RX_CLOSE.replace_all(&out, ")");
    let out = RX_UNDER.replace_all(&out, "_");
    let out = RX_COPY.replace_all(&out, "(c)");
    out.into()
}

fn trim_line(input: &str) -> String {
    input.trim().into()
}

// Aggressive preprocessors

fn lcs_substr(f_line: &str, s_line: &str) -> String {
    // grab character iterators from both strings
    let f_line_chars = f_line.chars();
    let s_line_chars = s_line.chars();

    // zip them together and find the common substring from the start
    f_line_chars
        .zip(s_line_chars)
        .take_while(|&(f, s)| f == s)
        .map(|(f, _s)| f)
        .collect::<String>()
        .trim()
        .into() //TODO: big optimization needed, this is a wasteful conversion
}

fn remove_common_tokens(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut l_iter = lines.iter().peekable();

    // TODO: consider whether this can all be done in one pass https://github.com/amzn/askalono/issues/36

    let mut prefix_counts: HashMap<String, u32> = HashMap::new();

    // pass 1: iterate through the text to record common prefixes
    while let Some(line) = l_iter.next() {
        if let Some(next) = l_iter.peek() {
            let common = lcs_substr(line, next);

            // why start at 1, then immediately add 1?
            // lcs_substr compares two lines!
            // this doesn't need to be exact, just consistent.
            if common.len() > 3 {
                *prefix_counts.entry(common.to_owned()).or_insert(1) += 1;
            }
        }
    }

    // look at the most common observed prefix
    let max_prefix = prefix_counts.iter().max_by_key(|&(_k, v)| v);
    if max_prefix.is_none() {
        return text.to_string();
    }
    let (most_common, _) = max_prefix.unwrap();

    // reconcile the count with other longer prefixes that may be stored
    let mut final_common_count = 0;
    for (k, v) in prefix_counts.iter() {
        if k.starts_with(most_common) {
            final_common_count += v;
        }
    }

    // the common string must be at least 80% of the text
    let prefix_threshold: u32 = (0.8f32 * lines.len() as f32) as u32;
    if final_common_count < prefix_threshold {
        return text.to_string();
    }

    // pass 2: remove that substring
    let prefix_len = most_common.len();
    lines
        .iter()
        .map(|line| {
            if line.starts_with(most_common) {
                &line[prefix_len..]
            } else {
                &line
            }
        })
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_vertical_whitespace(input: &str) -> String {
    lazy_static! {
        static ref RX_MISC: Regex = Regex::new(r"[\r\n\v\f]").unwrap();
        static ref RX_NUM: Regex = Regex::new(r"\n{3,}").unwrap();
    }
    let out = Cow::Borrowed(input);
    let out = RX_MISC.replace_all(&out, "\n");
    let out = RX_NUM.replace_all(&out, "\n\n");
    out.into()
}

fn remove_punctuation(input: &str) -> String {
    lazy_static! {
        static ref RX: Regex = Regex::new(r"[^\w\s]+").unwrap();
    }
    RX.replace_all(input, "").into()
}

fn lowercaseify(input: &str) -> String {
    input.to_lowercase()
}

fn remove_title_line(input: &str) -> String {
    lazy_static! {
        static ref RX: Regex = Regex::new(r"^.*license( version \S+)?( copyright.*)?\n\n").unwrap();
    }

    RX.replace_all(input, "").into()
}

fn remove_copyright_statements(input: &str) -> String {
    lazy_static! {
        static ref RX: Regex = Regex::new(
            r"(?mx)
            (
                # either a new paragraph, or the beginning of the text + empty lines
                (\n\n|\A\n*)
                # any number of lines starting with 'copyright' followed by a new paragraph
                (^\x20*copyright.*?$)+
                \n\n
            )
            |
            (
                # or the very first line if it has 'copyright' in it
                \A.*copyright.*$
            )
            |
            (
                # or any lines that really look like a copyright statement
                ^copyright (\s+(c|\d+))+ .*?$
            )
        "
        )
        .unwrap();
    }

    RX.replace_all(input, "\n\n").into()
}

fn collapse_whitespace(input: &str) -> String {
    lazy_static! {
        static ref RX: Regex = Regex::new(r"\s+").unwrap();
    }
    RX.replace_all(input, " ").into()
}

fn final_trim(input: &str) -> String {
    input.trim().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greatest_substring_removal() {
        // the funky string syntax \n\ is to add a newline but skip the
        // leading whitespace in the source code
        let text = "%%Copyright: Copyright\n\
                    %%Copyright: All rights reserved.\n\
                    %%Copyright: Redistribution and use in source and binary forms, with or\n\
                    %%Copyright: without modification, are permitted provided that the\n\
                    %%Copyright: following conditions are met:\n\
                    \n\
                    abcd";

        let new_text = remove_common_tokens(text);
        println!("{}", new_text);

        assert_eq!(
            new_text.contains("%%Copyright"),
            false,
            "new text shouldn't contain the common substring"
        );
    }

    #[test]
    fn greatest_substring_removal_keep_inner() {
        let text = "this string should still have\n\
                    this word -> this <- in it even though\n\
                    this is still the most common word";
        let new_text = remove_common_tokens(text);
        println!("-- {}", new_text);
        // the "this" at the start of the line can be discarded...
        assert!(!new_text.contains("\nthis"));
        // ...but the "this" in the middle of sentences shouldn't be
        assert!(new_text.contains("this"));

        let text = "aaaa bbbb cccc dddd\n\
                    eeee ffff aaaa gggg\n\
                    hhhh iiii jjjj";
        let new_text = remove_common_tokens(text);
        println!("-- {}", new_text);
        assert!(new_text.contains("aaaa")); // similar to above test
    }

    #[test]
    fn greatest_substring_removal_42() {
        // https://github.com/amzn/askalono/issues/42
        let text = "AAAAAA line 1\n\
                    AAAAAA another line here\n\
                    AAAAAA yet another line here\n\
                    AAAAAA how long will this go on\n\
                    AAAAAA another line here\n\
                    AAAAAA more\n\
                    AAAAAA one more\n\
                    AAAAAA two more\n\
                    AAAAAA three more\n\
                    AAAAAA four more\n\
                    AAAAAA five more\n\
                    AAAAAA six more\n\
                    \n\
                    preserve\n\
                    keep";
        let new_text = remove_common_tokens(text);
        println!("{}", new_text);

        assert!(new_text.contains("preserve"));
        assert!(new_text.contains("keep"));
        assert!(!new_text.contains("AAAAAA"));
    }

    #[test]
    fn normalize_no_line_mangle() {
        let text = "some license

        copyright 2012 person

        \tlicense\r
        text

        \t



        goes
        here";

        let text_lines = text.lines().count();

        let normalized = apply_normalizers(text);
        let normalized_lines = normalized.len();

        assert_eq!(
            text_lines, normalized_lines,
            "normalizers shouldnt change line counts"
        );
    }
}

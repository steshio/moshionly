//! Fractional indices: sort keys that always leave room between neighbours.
//!
//! Siblings in a document are ordered by a short string, compared byte by
//! byte. Putting a node between two others only needs a new key that sorts
//! between theirs; no other node has to change. That keeps moves and inserts
//! to a single edit and makes them safe to merge.
//!
//! A key is written in base 62 with the digits `0-9A-Za-z`, which is also
//! their ASCII order. It has an integer part and an optional fraction:
//!
//! - The first character of the integer part says how long it is. `a` to `z`
//!   announce 1 to 26 further digits and count upward; `Z` down to `A`
//!   announce 1 to 26 further digits and count downward, for keys before
//!   `a0`. So `a0`, `a1`, ... `az`, `b00`, `b01` are consecutive integers.
//! - The fraction is any number of digits after it and never ends in `0`.
//!   `a0V` sits between `a0` and `a1`.
//!
//! The first key of an empty list is `a0`.

/// The key given to the first node in an empty list.
pub const FIRST: &str = "a0";

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = DIGITS.len();

/// Why a key could not be used or generated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// The string is not a fractional index.
    #[error(
        "\"{0}\" is not a valid index; an index looks like \"a0\", \"a1\" or \"a0V\": \
         a letter, the digits it announces, and an optional fraction that does not end in 0"
    )]
    Invalid(String),
    /// The lower bound does not sort before the upper bound.
    #[error(
        "cannot make an index between \"{0}\" and \"{1}\": the first must sort before the second"
    )]
    NotOrdered(String, String),
    /// There is no key before the lower end or after the upper end of the key space.
    #[error("no index is left on that side of \"{0}\"")]
    Exhausted(String),
}

/// The value of a base-62 digit.
fn digit_value(digit: u8) -> Option<usize> {
    match digit {
        b'0'..=b'9' => Some((digit - b'0') as usize),
        b'A'..=b'Z' => Some((digit - b'A') as usize + 10),
        b'a'..=b'z' => Some((digit - b'a') as usize + 36),
        _ => None,
    }
}

/// The length of the integer part announced by its first character, itself included.
fn integer_len(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

/// Splits a valid key into its integer part and its fraction.
fn split(key: &str) -> Result<(&[u8], &[u8]), IndexError> {
    let invalid = || IndexError::Invalid(key.to_owned());
    let bytes = key.as_bytes();
    let head = *bytes.first().ok_or_else(invalid)?;
    let len = integer_len(head).ok_or_else(invalid)?;
    if bytes.len() < len || bytes.iter().any(|b| digit_value(*b).is_none()) {
        return Err(invalid());
    }
    let (integer, fraction) = bytes.split_at(len);
    if fraction.last() == Some(&b'0') {
        return Err(invalid());
    }
    // The very smallest integer has nothing before it, so it is reserved as a
    // lower bound and never used as a key on its own.
    if head == b'A' && fraction.is_empty() && integer[1..].iter().all(|b| *b == b'0') {
        return Err(invalid());
    }
    Ok((integer, fraction))
}

/// Checks that `key` is a well-formed fractional index.
pub fn validate(key: &str) -> Result<(), IndexError> {
    split(key).map(|_| ())
}

/// Whether `key` is a well-formed fractional index.
pub fn is_valid(key: &str) -> bool {
    validate(key).is_ok()
}

/// A fraction strictly between the fractions `low` and `high`, where `high`
/// of `None` means the open end above every fraction.
///
/// Fractions are compared as if padded with zeros, and never end in zero.
fn midpoint(mut low: &[u8], mut high: Option<&[u8]>) -> Vec<u8> {
    let mut result = Vec::new();
    loop {
        if let Some(upper) = high {
            // Copy the digits both sides share; the answer starts with them.
            let shared = upper
                .iter()
                .enumerate()
                .take_while(|(i, digit)| low.get(*i).copied().unwrap_or(b'0') == **digit)
                .count();
            if shared > 0 {
                result.extend_from_slice(&upper[..shared]);
                low = low.get(shared..).unwrap_or(&[]);
                high = Some(&upper[shared..]);
            }
        }

        let low_digit = low.first().and_then(|d| digit_value(*d)).unwrap_or(0);
        let high_digit = high
            .and_then(|upper| upper.first())
            .and_then(|d| digit_value(*d))
            .unwrap_or(BASE);

        if high_digit > low_digit + 1 {
            // Room for a digit in between: take the middle one.
            result.push(DIGITS[(low_digit + high_digit).div_ceil(2)]);
            return result;
        }
        if let Some(upper) = high
            && upper.len() > 1
        {
            // `high` continues past its first digit, so that digit alone is
            // already below it and above `low`.
            result.push(upper[0]);
            return result;
        }
        // The first digits are neighbours and `high` ends there: keep `low`'s
        // digit and look for room above the rest of `low`.
        result.push(DIGITS[low_digit]);
        low = low.get(1..).unwrap_or(&[]);
        high = None;
    }
}

/// The integer after `integer`, or `None` at the top of the key space.
fn increment(integer: &[u8]) -> Option<Vec<u8>> {
    let head = integer[0];
    let mut digits = integer[1..].to_vec();
    let mut carry = true;
    for digit in digits.iter_mut().rev() {
        let value = digit_value(*digit)? + 1;
        if value == BASE {
            *digit = b'0';
        } else {
            *digit = DIGITS[value];
            carry = false;
            break;
        }
    }
    let mut result = vec![head];
    if carry {
        match head {
            b'Z' => return Some(b"a0".to_vec()),
            b'z' => return None,
            // Above `a` every step up adds a digit; below it removes one.
            b'a'..=b'y' => digits.push(b'0'),
            _ => {
                digits.pop();
            }
        }
        result[0] = head + 1;
    }
    result.extend(digits);
    Some(result)
}

/// The integer before `integer`, or `None` at the bottom of the key space.
fn decrement(integer: &[u8]) -> Option<Vec<u8>> {
    let head = integer[0];
    let mut digits = integer[1..].to_vec();
    let mut borrow = true;
    for digit in digits.iter_mut().rev() {
        let value = digit_value(*digit)?;
        if value == 0 {
            *digit = b'z';
        } else {
            *digit = DIGITS[value - 1];
            borrow = false;
            break;
        }
    }
    let mut result = vec![head];
    if borrow {
        match head {
            b'a' => return Some(b"Zz".to_vec()),
            b'A' => return None,
            // Below `Z` every step down adds a digit; above it removes one.
            b'B'..=b'Z' => digits.push(b'z'),
            _ => {
                digits.pop();
            }
        }
        result[0] = head - 1;
    }
    result.extend(digits);
    Some(result)
}

fn into_key(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("keys are made of ASCII digits")
}

/// Generates a key that sorts strictly between `before` and `after`.
///
/// `None` on either side means "no neighbour on that side": `(None, None)`
/// gives the first key, `(Some(last), None)` a key after the last sibling,
/// `(None, Some(first))` a key before the first.
pub fn key_between(before: Option<&str>, after: Option<&str>) -> Result<String, IndexError> {
    let before = before
        .map(|key| split(key).map(|parts| (key, parts)))
        .transpose()?;
    let after = after
        .map(|key| split(key).map(|parts| (key, parts)))
        .transpose()?;
    if let (Some((low, _)), Some((high, _))) = (before, after)
        && low >= high
    {
        return Err(IndexError::NotOrdered(low.to_owned(), high.to_owned()));
    }

    let join = |integer: &[u8], fraction: Vec<u8>| {
        let mut key = integer.to_vec();
        key.extend(fraction);
        into_key(key)
    };

    match (before, after) {
        (None, None) => Ok(FIRST.to_owned()),
        (None, Some((high, (integer, fraction)))) => {
            if !fraction.is_empty() {
                // "a0V" has "a0" just below it. The reserved smallest integer
                // is not a key, so below it only smaller fractions are left.
                let below = into_key(integer.to_vec());
                return Ok(match split(&below) {
                    Ok(_) => below,
                    Err(_) => join(integer, midpoint(&[], Some(fraction))),
                });
            }
            match decrement(integer) {
                // The smallest integer is reserved; go just above it instead.
                Some(lower) if split(&into_key(lower.clone())).is_err() => {
                    Ok(join(&lower, midpoint(&[], None)))
                }
                Some(lower) => Ok(into_key(lower)),
                None => Err(IndexError::Exhausted(high.to_owned())),
            }
        }
        (Some((_, (integer, fraction))), None) => match increment(integer) {
            Some(higher) => Ok(into_key(higher)),
            // At the very top only fractions are left.
            None => Ok(join(integer, midpoint(fraction, None))),
        },
        (Some((_, (low_integer, low_fraction))), Some((high, (high_integer, high_fraction)))) => {
            if low_integer == high_integer {
                return Ok(join(
                    low_integer,
                    midpoint(low_fraction, Some(high_fraction)),
                ));
            }
            match increment(low_integer) {
                Some(next) if next.as_slice() < high.as_bytes() => Ok(into_key(next)),
                _ => Ok(join(low_integer, midpoint(low_fraction, None))),
            }
        }
    }
}

/// Generates `count` keys in ascending order, all strictly between `before`
/// and `after`.
///
/// The keys are spread evenly, so they stay short: appending many keys gives
/// consecutive integers, and filling a gap bisects it.
pub fn keys_between(
    before: Option<&str>,
    after: Option<&str>,
    count: usize,
) -> Result<Vec<String>, IndexError> {
    match (count, before, after) {
        (0, _, _) => Ok(Vec::new()),
        (1, _, _) => Ok(vec![key_between(before, after)?]),
        (_, _, None) => {
            let mut keys = Vec::with_capacity(count);
            let mut last = key_between(before, None)?;
            for _ in 1..count {
                let next = key_between(Some(&last), None)?;
                keys.push(std::mem::replace(&mut last, next));
            }
            keys.push(last);
            Ok(keys)
        }
        (_, None, Some(_)) => {
            let mut keys = Vec::with_capacity(count);
            let mut first = key_between(None, after)?;
            for _ in 1..count {
                let previous = key_between(None, Some(&first))?;
                keys.push(std::mem::replace(&mut first, previous));
            }
            keys.push(first);
            keys.reverse();
            Ok(keys)
        }
        (_, Some(_), Some(_)) => {
            let half = count / 2;
            let middle = key_between(before, after)?;
            let mut keys = keys_between(before, Some(&middle), half)?;
            let upper = keys_between(Some(&middle), after, count - half - 1)?;
            keys.push(middle);
            keys.extend(upper);
            Ok(keys)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_first_keys_are_short_and_consecutive() {
        assert_eq!(key_between(None, None).unwrap(), "a0");
        assert_eq!(key_between(Some("a0"), None).unwrap(), "a1");
        assert_eq!(key_between(Some("az"), None).unwrap(), "b00");
        assert_eq!(key_between(None, Some("a0")).unwrap(), "Zz");
        assert_eq!(key_between(None, Some("Z0")).unwrap(), "Yzz");
        assert_eq!(key_between(Some("a0"), Some("a1")).unwrap(), "a0V");
        assert_eq!(key_between(Some("a0"), Some("a0V")).unwrap(), "a0G");
        assert_eq!(key_between(Some("a0"), Some("a2")).unwrap(), "a1");
        assert_eq!(key_between(None, Some("a0V")).unwrap(), "a0");
        assert_eq!(
            keys_between(None, None, 12).unwrap(),
            [
                "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9", "aA", "aB"
            ]
        );
    }

    #[test]
    fn validation_accepts_keys_and_rejects_everything_else() {
        for good in [
            "a0",
            "a1",
            "az",
            "b00",
            "Zz",
            "a0V",
            "Yzz",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ] {
            assert!(is_valid(good), "{good} should be valid");
        }
        for bad in [
            "",
            "a",
            "0",
            "a00",
            "a0V0",
            "b0",
            "a-",
            "é0",
            "A00000000000000000000000000",
            "1a",
        ] {
            let error = validate(bad).unwrap_err();
            assert!(matches!(error, IndexError::Invalid(_)), "{bad}: {error}");
            assert!(
                error.to_string().contains("a0"),
                "the message shows an example: {error}"
            );
        }
    }

    #[test]
    fn bounds_must_be_valid_and_ordered() {
        assert!(matches!(
            key_between(Some("a1"), Some("a0")),
            Err(IndexError::NotOrdered(..))
        ));
        assert!(matches!(
            key_between(Some("a0"), Some("a0")),
            Err(IndexError::NotOrdered(..))
        ));
        assert!(matches!(
            key_between(Some("nope!"), None),
            Err(IndexError::Invalid(_))
        ));
        assert!(matches!(
            keys_between(Some("a1"), Some("a0"), 3),
            Err(IndexError::NotOrdered(..))
        ));
    }

    #[test]
    fn the_ends_of_the_key_space_still_produce_keys() {
        let top = "z".repeat(27);
        let above = key_between(Some(&top), None).unwrap();
        assert!(above.as_str() > top.as_str() && is_valid(&above));

        // Just above the reserved smallest integer.
        let lowest_usable = format!("A{}1", "0".repeat(25));
        let below = key_between(None, Some(&lowest_usable)).unwrap();
        assert!(
            below.as_str() < lowest_usable.as_str() && is_valid(&below),
            "{below}"
        );
        let lower_still = key_between(None, Some(&below)).unwrap();
        assert!(
            lower_still < below && is_valid(&lower_still),
            "{lower_still}"
        );
        // The bottom never runs out: every key still has a smaller one.
        let mut lowest = lower_still;
        for _ in 0..50 {
            let lower = key_between(None, Some(&lowest)).unwrap();
            assert!(lower < lowest && is_valid(&lower), "{lower}");
            lowest = lower;
        }
    }

    #[test]
    fn appending_and_prepending_a_thousand_times_stays_compact() {
        let mut last = FIRST.to_owned();
        let mut first = FIRST.to_owned();
        for _ in 0..1000 {
            let next = key_between(Some(&last), None).unwrap();
            assert!(next > last && is_valid(&next));
            last = next;
            let previous = key_between(None, Some(&first)).unwrap();
            assert!(previous < first && is_valid(&previous));
            first = previous;
        }
        assert!(last.len() <= 3, "{last}");
        assert!(first.len() <= 3, "{first}");
    }

    #[test]
    fn always_inserting_in_the_same_gap_grows_slowly() {
        let low = "a0".to_owned();
        let mut high = "a1".to_owned();
        for _ in 0..200 {
            let middle = key_between(Some(&low), Some(&high)).unwrap();
            assert!(low < middle && middle < high && is_valid(&middle));
            high = middle;
        }
        // Each digit halves the gap about six times.
        assert!(high.len() < 45, "{} characters", high.len());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Inserting at arbitrary positions keeps every key valid, distinct
        /// and in the order of the list.
        #[test]
        fn random_inserts_keep_the_list_ordered(positions in prop::collection::vec(0usize..10_000, 1..120)) {
            let mut keys: Vec<String> = Vec::new();
            for position in positions {
                let at = position % (keys.len() + 1);
                let before = at.checked_sub(1).map(|i| keys[i].as_str());
                let after = keys.get(at).map(String::as_str);
                let key = key_between(before, after).unwrap();
                prop_assert!(is_valid(&key), "{} is not valid", key);
                keys.insert(at, key);
            }
            for pair in keys.windows(2) {
                prop_assert!(pair[0] < pair[1], "{} should sort before {}", pair[0], pair[1]);
            }
        }

        /// A batch of keys lands strictly inside the gap, in order, without
        /// collisions.
        #[test]
        fn batches_fill_a_gap_in_order(
            setup in prop::collection::vec(0usize..1000, 0..30),
            gap in 0usize..1000,
            count in 0usize..40,
        ) {
            let mut keys: Vec<String> = Vec::new();
            for position in setup {
                let at = position % (keys.len() + 1);
                let before = at.checked_sub(1).map(|i| keys[i].as_str());
                let after = keys.get(at).map(String::as_str);
                keys.insert(at, key_between(before, after).unwrap());
            }
            let at = gap % (keys.len() + 1);
            let before = at.checked_sub(1).map(|i| keys[i].as_str());
            let after = keys.get(at).map(String::as_str);
            let batch = keys_between(before, after, count).unwrap();
            prop_assert_eq!(batch.len(), count);
            let mut previous = before.map(str::to_owned);
            for key in &batch {
                prop_assert!(is_valid(key));
                if let Some(previous) = &previous {
                    prop_assert!(previous < key, "{} should sort before {}", previous, key);
                }
                previous = Some(key.clone());
            }
            if let (Some(last), Some(after)) = (batch.last(), after) {
                prop_assert!(last.as_str() < after);
            }
        }

        /// Arbitrary strings never panic the validator or the generator.
        #[test]
        fn arbitrary_input_is_rejected_not_fatal(a in ".{0,12}", b in ".{0,12}") {
            let _ = validate(&a);
            let _ = key_between(Some(&a), Some(&b));
            let _ = keys_between(Some(&a), None, 3);
        }
    }
}

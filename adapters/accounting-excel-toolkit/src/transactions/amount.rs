//! Scale-preserving coefficients adapted from the grant workpaper's Amount.
use super::text::digit;

#[derive(Clone, Copy, Debug)]
pub struct Amount {
    coefficient: i128,
    places: u32,
}

impl Amount {
    pub const ZERO: Self = Self {
        coefficient: 0,
        places: 0,
    };

    fn aligned(self, places: u32) -> i128 {
        self.coefficient * 10i128.pow(places - self.places)
    }

    pub fn add(self, other: Self) -> Result<Self, String> {
        let places = self.places.max(other.places);
        let coefficient = self
            .aligned(places)
            .checked_add(other.aligned(places))
            .ok_or("Amount total exceeds the native integer bound.")?;
        Ok(Self {
            coefficient,
            places,
        })
    }

    pub fn positive(self) -> bool {
        self.coefficient > 0
    }
}

impl PartialEq for Amount {
    fn eq(&self, other: &Self) -> bool {
        let places = self.places.max(other.places);
        self.aligned(places) == other.aligned(places)
    }
}
impl Eq for Amount {}

impl std::fmt::Display for Amount {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let places = self.places as usize;
        let digits = format!("{:0>width$}", self.coefficient, width = places + 1);
        if places == 0 {
            return out.write_str(&digits);
        }
        let (whole, fraction) = digits.split_at(digits.len() - places);
        write!(out, "{whole}.{fraction}")
    }
}

pub fn parse(text: &str) -> Result<Amount, String> {
    let invalid = || {
        format!(
            "Expected an explicit non-negative amount with at most 2 places: {}",
            serde_json::to_string(text).expect("string")
        )
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let groups: Vec<_> = whole.split(',').collect();
    let sizes: Vec<_> = groups.iter().map(|group| group.chars().count()).collect();
    let valid_whole = if groups.len() == 1 {
        (1..=15).contains(&sizes[0])
    } else {
        (2..=5).contains(&groups.len())
            && (1..=3).contains(&sizes[0])
            && sizes[1..].iter().all(|&n| n == 3)
    };
    let places = fraction.chars().count();
    if !valid_whole || places > 2 || (text.contains('.') && places == 0) {
        return Err(invalid());
    }
    let digits = whole
        .chars()
        .filter(|&ch| ch != ',')
        .chain(fraction.chars())
        .map(digit)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid)?;
    let coefficient = digits.iter().fold(0i128, |n, &d| n * 10 + i128::from(d));
    Ok(Amount {
        coefficient,
        places: places as u32,
    })
}

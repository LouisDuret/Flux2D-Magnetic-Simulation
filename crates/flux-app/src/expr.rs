//! Saisie d'expressions dans les champs numériques : « 12 mm + 3 mm », « 2 * (1 cm + 0,5) »,
//! « 45° », « 300 K ». Le résultat est exprimé dans l'unité affichée par le champ.

/// Grandeur d'un champ numérique. Elle fixe l'unité affichée et les unités acceptées à la saisie.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Quantity {
    /// Affichée en millimètres, stockée en mètres.
    Length,
    /// Affiché en degrés, stocké en radians.
    Angle,
    /// En degrés Celsius.
    Temperature,
    /// En ampères.
    Current,
    /// Sans unité.
    Count,
}

impl Quantity {
    /// Facteur de la valeur stockée (SI) vers la valeur affichée.
    pub fn factor(self) -> f64 {
        match self {
            Quantity::Length => 1e3,
            Quantity::Angle => 180.0 / std::f64::consts::PI,
            _ => 1.0,
        }
    }

    /// Convertit `x`, suivi de l'unité `unit`, dans l'unité affichée. Sans unité, `x` y est déjà.
    fn convert(self, x: f64, unit: &str) -> Option<f64> {
        if unit.is_empty() {
            return Some(x);
        }
        Some(match (self, unit) {
            (Quantity::Length, "mm") => x,
            (Quantity::Length, "cm") => x * 10.0,
            (Quantity::Length, "m") => x * 1e3,
            (Quantity::Length, "µm" | "μm" | "um") => x * 1e-3,
            (Quantity::Length, "in" | "po") => x * 25.4,
            (Quantity::Angle, "°" | "deg") => x,
            (Quantity::Angle, "rad") => x.to_degrees(),
            (Quantity::Angle, "tr") => x * 360.0,
            (Quantity::Temperature, "°C" | "C") => x,
            (Quantity::Temperature, "K") => x - 273.15,
            (Quantity::Temperature, "°F" | "F") => (x - 32.0) / 1.8,
            (Quantity::Current, "A") => x,
            (Quantity::Current, "mA") => x * 1e-3,
            (Quantity::Current, "kA") => x * 1e3,
            _ => return None,
        })
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    quantity: Quantity,
}

impl Parser<'_> {
    fn skip_spaces(&mut self) {
        while self.chars.next_if(|c| c.is_whitespace()).is_some() {}
    }

    /// Consomme le caractère suivant s'il fait partie de `set` (espaces ignorés).
    fn eat(&mut self, set: &str) -> Option<char> {
        self.skip_spaces();
        self.chars.next_if(|c| set.contains(*c))
    }

    /// expression = terme (('+' | '-') terme)*
    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(op) = self.eat("+-−") {
            let rhs = self.term()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    /// terme = facteur (('*' | '/') facteur)*
    fn term(&mut self) -> Option<f64> {
        let mut value = self.factor()?;
        while let Some(op) = self.eat("*×/") {
            let rhs = self.factor()?;
            value = if op == '/' { value / rhs } else { value * rhs };
        }
        Some(value)
    }

    /// facteur = ('+' | '-') facteur | '(' expression ')' | nombre unité?
    fn factor(&mut self) -> Option<f64> {
        if let Some(sign) = self.eat("+-−") {
            return self.factor().map(|v| if sign == '+' { v } else { -v });
        }
        if self.eat("(").is_some() {
            let value = self.expression()?;
            return self.eat(")").map(|_| value);
        }
        // Nombre : virgule ou point décimal, exposant facultatif.
        let mut text = String::new();
        while let Some(c) = self.chars.next_if(|c| c.is_ascii_digit() || *c == '.' || *c == ',') {
            text.push(if c == ',' { '.' } else { c });
        }
        let mut ahead = self.chars.clone();
        if matches!(ahead.next(), Some('e' | 'E')) {
            let sign = ahead.next_if(|c| *c == '+' || *c == '-');
            if ahead.peek().is_some_and(char::is_ascii_digit) {
                text.push('e');
                text.extend(sign);
                self.chars = ahead;
                while let Some(c) = self.chars.next_if(char::is_ascii_digit) {
                    text.push(c);
                }
            }
        }
        let number: f64 = text.parse().ok()?;
        self.skip_spaces();
        let mut unit = String::new();
        while let Some(c) = self.chars.next_if(|c| c.is_alphabetic() || *c == '°') {
            unit.push(c);
        }
        self.quantity.convert(number, &unit)
    }
}

/// Évalue une expression saisie dans un champ de grandeur `quantity`. Renvoie la valeur dans
/// l'unité affichée par le champ, ou `None` si le texte n'est pas une expression valide.
pub fn eval(text: &str, quantity: Quantity) -> Option<f64> {
    let mut parser = Parser { chars: text.chars().peekable(), quantity };
    let value = parser.expression()?;
    parser.skip_spaces();
    (parser.chars.next().is_none() && value.is_finite()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::Quantity::*;
    use super::*;

    fn close(text: &str, quantity: Quantity, want: f64) {
        let got = eval(text, quantity).unwrap_or_else(|| panic!("« {text} » devrait être valide"));
        assert!((got - want).abs() < 1e-9 * want.abs().max(1.0), "« {text} » = {got} au lieu de {want}");
    }

    #[test]
    fn lengths_in_millimeters() {
        close("12 mm + 3 mm", Length, 15.0);
        close("12,5", Length, 12.5);
        close("1 cm + 2", Length, 12.0);
        close("0.02 m", Length, 20.0);
        close("2 * (1cm + 0,5) / 4", Length, 5.25);
        close("250 µm", Length, 0.25);
        close("1 in", Length, 25.4);
        close("-3 + 10", Length, 7.0);
        close("1e-2 m", Length, 10.0);
        close("2 × 3", Length, 6.0);
        close("10 − 4", Length, 6.0);
    }

    #[test]
    fn other_quantities() {
        close("45°", Angle, 45.0);
        close("3.14159265358979 rad / 2", Angle, 90.0);
        close("0,25 tr", Angle, 90.0);
        close("300 K", Temperature, 26.85);
        close("-196 °C", Temperature, -196.0);
        close("212 °F", Temperature, 100.0);
        close("500 mA * 2", Current, 1.0);
        close("100 + 20", Count, 120.0);
    }

    #[test]
    fn rejects_nonsense() {
        for (text, quantity) in [
            ("", Length),
            ("abc", Length),
            ("12 mm +", Length),
            ("(3", Length),
            ("3 kg", Length),
            ("5 mm", Angle),
            ("1 / 0", Count),
            ("2 mm 3", Length),
            ("4 mm", Count),
        ] {
            assert_eq!(eval(text, quantity), None, "« {text} » devrait être refusé");
        }
    }
}

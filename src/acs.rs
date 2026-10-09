//! Access Condition Strings, ported from `aacs1` in COMMON.PAS.
//!
//! An ACS is a run of terms like `s20`, `fA`, `!rP`. Terms next to each other
//! (or joined by `&`) are ANDed, `|` ORs, `!` negates, `()` groups. As in the
//! original, `|` binds tighter than AND: `s10s20|fA` means `s10 & (s20 | fA)`.
//! An empty ACS always passes.

/// Letters whose argument is a single character rather than a number.
const CHAR_ARG: &str = "CFGRVX";

/// Evaluate `acs`, asking `term(letter, arg)` for the value of each term.
pub fn eval(acs: &str, term: &mut dyn FnMut(char, &str) -> bool) -> bool {
    let mut p = Parser { s: acs.to_ascii_uppercase().chars().collect(), i: 0, term };
    p.and()
}

struct Parser<'a> {
    s: Vec<char>,
    i: usize,
    term: &'a mut dyn FnMut(char, &str) -> bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some('&' | ' ')) {
            self.i += 1;
        }
    }

    fn and(&mut self) -> bool {
        let mut r = true;
        loop {
            self.skip();
            match self.peek() {
                None => return r,
                Some(')') => {
                    self.i += 1;
                    return r;
                }
                Some('|') => self.i += 1, // dangling OR, ignored like the original
                _ => r &= self.or(),
            }
        }
    }

    fn or(&mut self) -> bool {
        let mut r = self.atom();
        loop {
            self.skip();
            if self.peek() != Some('|') {
                return r;
            }
            self.i += 1;
            self.skip();
            r |= self.atom();
        }
    }

    fn atom(&mut self) -> bool {
        let neg = self.peek() == Some('!');
        if neg {
            self.i += 1;
        }
        let v = match self.peek() {
            None | Some(')') => true,
            Some('(') => {
                self.i += 1;
                self.and()
            }
            Some(c) if c.is_ascii_uppercase() => {
                self.i += 1;
                let mut arg = String::new();
                if CHAR_ARG.contains(c) {
                    if let Some(a) = self.peek() {
                        arg.push(a);
                        self.i += 1;
                    }
                } else {
                    while let Some(d) = self.peek().filter(char::is_ascii_digit) {
                        arg.push(d);
                        self.i += 1;
                    }
                }
                (self.term)(c, &arg)
            }
            Some(_) => {
                self.i += 1;
                true
            }
        };
        v != neg
    }
}

#[cfg(test)]
mod tests {
    use super::eval;

    /// A user with SL 50 and AR flags A and C.
    fn user(acs: &str) -> bool {
        eval(acs, &mut |c, arg| match c {
            'S' => 50 >= arg.parse().unwrap_or(0),
            'F' => "AC".contains(arg),
            _ => true,
        })
    }

    #[test]
    fn basics() {
        assert!(user(""));
        assert!(user("s50"));
        assert!(!user("s51"));
        assert!(user("!s51"));
        assert!(user("fA"));
        assert!(!user("fB"));
        assert!(!user("s10fB"));
        assert!(user("s10&fC"));
        assert!(user("s99|fA"));
        assert!(!user("s99|fB"));
        assert!(user("S10 & (s99|fc)"));
        assert!(!user("!(s10fA)"));
        // OR binds tighter than AND, as in Impulse.
        assert!(!user("s99fB|fA"));
        assert!(user("s10s99|fA"));
    }
}

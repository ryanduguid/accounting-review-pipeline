// Fx.ABNIsValid
// Validate an Australian Business Number using the ATO checksum algorithm:
// subtract 1 from the first digit, multiply the 11 digits by weights
// (10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19), sum, and check mod 89 = 0.
//
// Accepts formatted input ("51 824 753 556") and number-typed columns.
// Spaces are the only formatting characters accepted: silently stripping
// letters or punctuation could turn a malformed identifier into a different,
// checksum-valid ABN. Values that Power Query cannot convert to text also
// return false rather than breaking a whole refresh.
// Returns false for null, malformed formatting, anything that isn't 11 digits
// after removing spaces, and for a leading zero: no ABN starts with 0, and the
// checksum alone can't reject one ("00000090000" sums to exactly 89).
// Checksum validity does not mean the ABN is registered or active. Check
// ABN Lookup (abr.business.gov.au) for registration status.
//
// Usage: Fx.ABNIsValid("51 824 753 556")  // true (ATO's published example)
let
    ABNIsValid = (abn as any) as logical =>
        let
            textAttempt = try Text.From(abn ?? "", "en-AU"),
            // Validate the converted value before normalisation. Text.Trim also
            // removes tabs, newlines, and other whitespace that this contract rejects.
            raw = if textAttempt[HasError] then "" else textAttempt[Value],
            allowedFormatting = Text.Select(raw, {"0".."9", " "}) = raw,
            digitsOnly = Text.Remove(raw, {" "}),
            result =
                if not allowedFormatting
                    or Text.Length(digitsOnly) <> 11
                    or Text.Select(digitsOnly, {"0".."9"}) <> digitsOnly
                    or Text.StartsWith(digitsOnly, "0") then
                    false
                else
                    let
                        digits = List.Transform(Text.ToList(digitsOnly), Number.FromText),
                        adjusted = {digits{0} - 1} & List.Skip(digits, 1),
                        weights = {10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19},
                        total = List.Sum(
                            List.Transform({0..10}, each adjusted{_} * weights{_})
                        )
                    in
                        Number.Mod(total, 89) = 0
        in
            result
in
    ABNIsValid

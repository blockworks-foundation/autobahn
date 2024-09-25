use solana_program::pubkey::Pubkey;
use solana_sdk::address_lookup_table::AddressLookupTableAccount;

pub fn get_best_alt(
    all_alt: &Vec<AddressLookupTableAccount>,
    tx_addresses: &Vec<Pubkey>,
) -> anyhow::Result<Vec<AddressLookupTableAccount>> {
    get_best_alt_internal(all_alt, tx_addresses, 0)
}

pub fn get_best_alt_internal(
    all_alt: &Vec<AddressLookupTableAccount>,
    tx_addresses: &Vec<Pubkey>,
    level: u8,
) -> anyhow::Result<Vec<AddressLookupTableAccount>> {
    let mut sorted_all_alt = all_alt
        .iter()
        .map(|alt| {
            (
                alt,
                tx_addresses
                    .iter()
                    .filter(|tx_address| alt.addresses.contains(tx_address))
                    .count(),
            )
        })
        .collect::<Vec<_>>();

    sorted_all_alt.sort_by_key(|alt| std::cmp::Reverse(alt.1));

    if sorted_all_alt.is_empty() || sorted_all_alt[0].1 <= 1 {
        // Only use LUT if it replaces 2 or more addr
        return Ok(vec![]);
    }

    let result = sorted_all_alt[0..1]
        .iter()
        .map(|x| x.0.clone())
        .collect::<Vec<_>>();

    if level < 3 {
        sorted_all_alt.remove(0);
        let all_alt = sorted_all_alt.into_iter().map(|x| x.0.clone()).collect();
        let tx_addresses = tx_addresses
            .into_iter()
            .filter(|x| !result[0].addresses.contains(x))
            .copied()
            .collect();

        let next = get_best_alt_internal(&all_alt, &tx_addresses, level + 1)?;
        let result = result.into_iter().chain(next.into_iter()).collect();
        return Ok(result);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use itertools::Itertools;

    #[test]
    fn should_find_best_alt() {
        let addr = (1..10i32).map(|_| Pubkey::new_unique()).collect::<Vec<_>>();
        let alt0 = make_alt(&[&addr[0], &addr[1]]);
        let alt1 = make_alt(&[&addr[2]]);
        let alt2 = make_alt(&[]);
        let alt3 = make_alt(&[&addr[3], &addr[4], &addr[5]]);
        let alts = vec![alt0, alt1, alt2, alt3];

        assert_alt_are(&addr[0..3], &alts, &[alts[0].clone()]);
        assert_alt_are(&addr[2..3], &alts, &[]);
        assert_alt_are(&addr[3..7], &alts, &[alts[3].clone()]);
        assert_alt_are(&addr[7..9], &alts, &[]);
        assert_alt_are(&addr[0..8], &alts, &[alts[0].clone(), alts[3].clone()]);
    }

    fn assert_alt_are(
        tx_addresses: &[Pubkey],
        all_alts: &Vec<AddressLookupTableAccount>,
        expected_alts: &[AddressLookupTableAccount],
    ) {
        let result = get_best_alt(&all_alts, &tx_addresses.iter().copied().collect()).unwrap();

        assert_eq!(
            result.iter().map(|x| x.key.to_string()).sorted().join("; "),
            expected_alts
                .iter()
                .map(|x| x.key.to_string())
                .sorted()
                .join("; "),
        );
    }

    fn make_alt(addresses: &[&Pubkey]) -> AddressLookupTableAccount {
        AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses: addresses.iter().map(|x| **x).collect(),
        }
    }
}

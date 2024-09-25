pub fn write_ux16(data: &mut [u8], n: u16) -> usize {
    if n >= 255 {
        data[0] = 255;
        1 + write_ux16(&mut data[1..], n - 255)
    } else {
        data[0] = n as u8;
        1
    }
}

pub fn write_bytes(data: &mut [u8], b: &[u8]) -> usize {
    data[0..b.len()].copy_from_slice(b);
    b.len()
}

pub fn write_u64(data: &mut [u8], n: u64) -> usize {
    data[0..8].copy_from_slice(n.to_le_bytes().as_slice());
    8
}

pub fn write_u8(data: &mut [u8], n: u8) -> usize {
    data[0] = n;
    1
}

/// Read an optimized u16 from one or N u8
/// -> if data[0] is < 255 -> return it
/// -> else, return 255+data[1]
/// (can recurse and use more than two u8 but should never happen)
pub fn read_ux16(data: &[u8]) -> (u16, &[u8]) {
    let x = data[0] as u16;
    if x < u8::MAX as u16 {
        (x, &data[1..])
    } else {
        let (y, rest) = read_ux16(&data[1..]);
        (x + y, rest)
    }
}

pub fn read_bytes(size: usize, data: &[u8]) -> (&[u8], &[u8]) {
    extract_part(data, size, |x| x)
}

pub fn read_u64(data: &[u8]) -> (u64, &[u8]) {
    extract_part(data, 8, |x| u64::from_le_bytes(x.try_into().unwrap()))
}

pub fn read_u8(data: &[u8]) -> (u8, &[u8]) {
    extract_part(data, 1, |x| x[0])
}

pub fn extract_part<'s, F: FnOnce(&'s [u8]) -> T, T>(
    data: &'s [u8],
    size: usize,
    transformer: F,
) -> (T, &[u8]) {
    let part = &data[0..size];
    let rest = &data[size..];

    (transformer(part), rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test]
    fn should_correctly_encode_and_decode_data() {
        let mut data = [0; 100];

        assert_eq!(write_u8(&mut data[0..], 27), 1);
        assert_eq!(write_u8(&mut data[1..], 42), 1);
        assert_eq!(write_u64(&mut data[2..], 123_456_789), 8);
        assert_eq!(write_u64(&mut data[10..], 100_000_000_000), 8);
        assert_eq!(write_ux16(&mut data[18..], 12), 1);
        assert_eq!(write_ux16(&mut data[19..], 200), 1);
        assert_eq!(write_ux16(&mut data[20..], 300), 2);
        assert_eq!(write_ux16(&mut data[22..], 600), 3);
        assert_eq!(write_bytes(&mut data[25..], "azerty".as_bytes()), 6);

        assert_eq!(read_u8(&data[0..]).0, 27);
        assert_eq!(read_u8(&data[1..]).0, 42);
        assert_eq!(read_u64(&data[2..]).0, 123_456_789);
        assert_eq!(read_u64(&data[10..]).0, 100_000_000_000);
        assert_eq!(read_ux16(&data[18..]).0, 12);
        assert_eq!(read_ux16(&data[19..]).0, 200);
        assert_eq!(read_ux16(&data[20..]).0, 300);
        assert_eq!(read_ux16(&data[22..]).0, 600);
        assert_eq!(read_bytes(6, &data[25..]).0, "azerty".as_bytes());
    }

    #[test_case(0, 1)]
    #[test_case(1, 1)]
    #[test_case(10, 1)]
    #[test_case(50, 1)]
    #[test_case(250, 1)]
    #[test_case(300, 2)]
    #[test_case(500, 2)]
    #[test_case(700, 3)]
    fn should_encode_decode_ux16(n: u16, expected_size: usize) {
        let mut data = [0; 100];

        assert_eq!(write_ux16(&mut data, n), expected_size);
        assert_eq!(read_ux16(&data).0, n);
    }
}

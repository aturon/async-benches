#![feature(test)]

extern crate test;
use test::Bencher;

#[derive(Copy, Clone)]
enum TwoState<B> {
    State1(B),
    State2,
}

impl<B> TwoState<B> {
    fn new(b: B) -> TwoState<B> {
        TwoState::State1(b)
    }
}

#[derive(Copy, Clone)]
enum SixteenState {
    S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S15, S16,
}

#[derive(Copy, Clone)]
struct Cap;

struct IsDone(bool);

trait NextState {
    fn next_state(&mut self) -> IsDone;
}

impl NextState for Cap {
    fn next_state(&mut self) -> IsDone {
        IsDone(true)
    }
}

impl<B: NextState> NextState for TwoState<B> {
    fn next_state(&mut self) -> IsDone {
        let transition = match *self {
            TwoState::State1(ref mut b) => {
                b.next_state().0
            }
            TwoState::State2 => {
                return IsDone(true)
            }
        };
        if transition {
            *self = TwoState::State2;
        }
        IsDone(test::black_box(false))
    }
}

impl NextState for SixteenState {
    fn next_state(&mut self) -> IsDone {
        use SixteenState::*;

        let (s, f) = match *self {
            S1 => (S2, false),
            S2 => (S3, false),
            S3 => (S4, false),
            S4 => (S5, false),
            S5 => (S6, false),
            S6 => (S7, false),
            S7 => (S8, false),
            S8 => (S9, false),
            S9 => (S10, false),
            S10 => (S11, false),
            S11 => (S12, false),
            S12 => (S13, false),
            S13 => (S14, false),
            S14 => (S15, false),
            S15 => (S16, false),
            S16 => (S16, true),
        };

        *self = s;
        IsDone(test::black_box(f))
    }
}

#[bench]
fn bench_setup(b: &mut Bencher) {
    let nested = TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(Cap))))))))))))))));

    let flat = SixteenState::S1;

    b.iter(|| {
        let mut nested = test::black_box(nested).clone();
        let mut flat = test::black_box(flat).clone();
        test::black_box((nested, flat))
    })
}

#[bench]
fn bench_sixteen_states_nested(b: &mut Bencher) {
    let nested = TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(Cap))))))))))))))));

    let flat = SixteenState::S1;

    b.iter(|| {
        let mut nested = test::black_box(nested).clone();
        let mut flat = test::black_box(flat).clone();
        while !nested.next_state().0 {}
    })
}

#[bench]
fn bench_sixteen_states_flat(b: &mut Bencher) {
    let nested = TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(TwoState::new(Cap))))))))))))))));

    let flat = SixteenState::S1;

    b.iter(|| {
        let mut nested = test::black_box(nested).clone();
        let mut flat = test::black_box(flat).clone();
        while !flat.next_state().0 {}
    })
}

fn main() {}

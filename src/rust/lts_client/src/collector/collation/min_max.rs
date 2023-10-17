use num_traits::{Bounded, CheckedDiv, NumCast, Zero};

#[derive(Debug, Clone)]
pub(crate) struct MinMaxAvg<T> {
  pub(crate) min: T,
  pub(crate) max: T,
  pub(crate) avg: T,
}

impl<
    T: Bounded
      + Zero
      + std::ops::AddAssign<T>
      + Copy
      + std::cmp::Ord
      + CheckedDiv
      + NumCast,
  > MinMaxAvg<T>
{
  pub(crate) fn from_slice(stats: &[T]) -> Self {
    let mut min = T::max_value();
    let mut max = T::min_value();
    let mut avg = T::zero();

    stats.iter().for_each(|n| {
      avg += *n;
      min = T::min(min, *n);
      max = T::max(max, *n);
    });
    let len = T::from(stats.len()).unwrap();
    avg = avg.checked_div(&len).unwrap_or(T::zero());

    Self { max, min, avg }
  }
}

#[derive(Debug, Clone)]
pub(crate) struct MinMaxAvgPair<T> {
  pub(crate) down: MinMaxAvg<T>,
  pub(crate) up: MinMaxAvg<T>,
}

impl<
    T: Bounded
      + Zero
      + std::ops::AddAssign<T>
      + Copy
      + std::cmp::Ord
      + CheckedDiv
      + NumCast,
  > MinMaxAvgPair<T>
{
pub(crate) fn from_slice(stats: &[(T, T)]) -> Self {
    let down: Vec<T> = stats.iter().map(|(down, _up)| *down).collect();
    let up: Vec<T> = stats.iter().map(|(_down, up)| *up).collect();
    Self {
      down: MinMaxAvg::<T>::from_slice(&down),
      up: MinMaxAvg::<T>::from_slice(&up),
    }
  }
}

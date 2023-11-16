use num_traits::{Bounded, CheckedDiv, NumCast, Zero};

#[derive(Debug, Clone)]
pub(crate) struct MinMaxAvg<T> {
  pub(crate) min: T,
  pub(crate) max: T,
  pub(crate) avg: T,
}

fn median<T>(stats: &[T]) -> T
where
  T: Bounded + Zero + std::ops::AddAssign<T> + Copy + std::cmp::Ord + CheckedDiv + NumCast,
{
  let mut sorted = stats.to_vec();
  sorted.sort();
  let len = sorted.len();
  let mid = len / 2;
  if len % 2 == 0 {
    (sorted[mid] + sorted[mid - 1]).checked_div(&T::from(2).unwrap()).unwrap_or(T::zero())
  } else {
    sorted[mid]
  }
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

    stats.iter().for_each(|n| {
      min = T::min(min, *n);
      max = T::max(max, *n);
    });
    let mut values = stats.to_vec();
    values.sort();
    let length = values.len();
    let median = if length == 0 { T::zero() } else {
      median(&values)
    };

    Self { max, min, avg: median }
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

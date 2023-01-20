use std::{time::Instant, str::from_utf8};

use rtnetlink::{new_connection, packet::{TcMessage, tc::{Nla, Stats2}}};
use futures_util::TryStreamExt;

#[allow(non_camel_case_types)]
#[derive(Debug)]
enum TcaCakeStats {
	__TCA_CAKE_STATS_INVALID = 0,
	TCA_CAKE_STATS_PAD = 1,
	TCA_CAKE_STATS_CAPACITY_ESTIMATE64 = 2,
	TCA_CAKE_STATS_MEMORY_LIMIT = 3,
	TCA_CAKE_STATS_MEMORY_USED =4,
	TCA_CAKE_STATS_AVG_NETOFF =5,
	TCA_CAKE_STATS_MIN_NETLEN =6,
	TCA_CAKE_STATS_MAX_NETLEN =7,
	TCA_CAKE_STATS_MIN_ADJLEN =8,
	TCA_CAKE_STATS_MAX_ADJLEN =9,
	TCA_CAKE_STATS_TIN_STATS =10,
	TCA_CAKE_STATS_DEFICIT =11,
	TCA_CAKE_STATS_COBALT_COUNT=12,
	TCA_CAKE_STATS_DROPPING=13,
	TCA_CAKE_STATS_DROP_NEXT_US=14,
	TCA_CAKE_STATS_P_DROP=15,
	TCA_CAKE_STATS_BLUE_TIMER_US=16,
	__TCA_CAKE_STATS_MAX=17
}

#[tokio::main]
async fn main() -> Result<(), ()> {
  let (connection, handle, _) = new_connection().unwrap();
  tokio::spawn(connection);

  let now = Instant::now();
  let mut result = handle.qdisc().get().index(3).execute();
  let elapsed = now.elapsed();
  println!("Time spent in NetLink API: {} nanoseconds", elapsed.as_nanos());
  let mut results = Vec::new();
  while let Ok(Some(result)) = result.try_next().await {
    results.push(result);
  }
  println!("Retrieved {} messages", results.len());
  println!("Time spent in NetLink API: {} nanoseconds", elapsed.as_nanos());

  let mut i = 0;
  loop {
    let msg = format!("{:?}", results[i]);
    if msg.contains("cake") {
      //println!("{msg}");
      for nlas in results[i].nlas.iter() {
        match nlas {
          Nla::Stats2(o) => {
            for stats in o.iter() {
              match stats {
                Stats2::StatsApp(o) => {
                  println!("{:?}", o);

                  let mut count = 0;
                  while count < o.len() {
                    let field: TcaCakeStats = o[count].into();
                    count += std::mem::sizeof<u32>();
                  }
                },
                _ => {}
              }
            }
          }
          _ => {}
        }
      }
      break;
    }
    i+=1;
  }

  Ok(())
}
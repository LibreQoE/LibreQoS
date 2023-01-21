use std::{time::Instant, str::from_utf8};

use derivative::Derivative;
use rtnetlink::{new_connection, packet::{TcMessage, tc::{Nla, Stats2}}};
use futures_util::TryStreamExt;

#[allow(non_camel_case_types)]
#[derive(Debug)]
#[repr(u8)]
enum TcaCakeStats {
	TCA_CAKE_STATS_INVALID = 0,
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
	TCA_CAKE_STATS_MAX=17
}

impl From<u8> for TcaCakeStats {
  fn from(n: u8) -> Self {
      match n {
        0 => Self::TCA_CAKE_STATS_INVALID,
        1 => Self::TCA_CAKE_STATS_PAD,
        2 => Self::TCA_CAKE_STATS_CAPACITY_ESTIMATE64,
        3 => Self::TCA_CAKE_STATS_MEMORY_LIMIT,
        4 => Self::TCA_CAKE_STATS_MEMORY_USED,
        5 => Self::TCA_CAKE_STATS_AVG_NETOFF,
        6 => Self::TCA_CAKE_STATS_MIN_NETLEN,
        7 => Self::TCA_CAKE_STATS_MAX_NETLEN,
        8 => Self::TCA_CAKE_STATS_MIN_ADJLEN,
        9 => Self::TCA_CAKE_STATS_MAX_ADJLEN,
        10 => Self::TCA_CAKE_STATS_TIN_STATS,
        11 => Self::TCA_CAKE_STATS_DEFICIT,
        12 => Self::TCA_CAKE_STATS_COBALT_COUNT,
        13 => Self::TCA_CAKE_STATS_DROPPING,
        14 => Self::TCA_CAKE_STATS_DROP_NEXT_US,
        15 => Self::TCA_CAKE_STATS_P_DROP,
        16 => Self::TCA_CAKE_STATS_BLUE_TIMER_US,
        17 => Self::TCA_CAKE_STATS_MAX,
        _ => Self::TCA_CAKE_STATS_INVALID,
      }
  }
}

fn slice_to_num(buff: &[u8]) -> u32 {
  u32::from_ne_bytes(
      buff.try_into().unwrap())
}

fn slice_to_u16(buff: &[u8]) -> u16 {
  u16::from_ne_bytes(
      buff.try_into().unwrap())
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
      println!("{msg}");
      for nlas in results[i].nlas.iter() {
        match nlas {
          Nla::Stats2(o) => {
            for stats in o.iter() {
              match stats {
                Stats2::StatsApp(o) => {
                  println!("{:?}", o);

                  let size = std::mem::size_of::<CakeStats2>();
                  println!("Size: {size} (vs buffer size {}", o.len());
                  let data = bytemuck::cast_slice::<u8, CakeStats2>(&o[0 .. size])[0];
                  println!("{:#?}", data);

                  /*let mut count = 0;
                  while count < o.len() {
                    //let field: TcaCakeStats = TcaCakeStats::from(o[count]);
                    println!("{}", slice_to_num(&o[count .. count+4]) );
                    count += 5;
                  }*/
                },
                _ => {}
              }
            }
          }
          _ => {}
        }
      }
      //break;
    }
    i+=1;
    if i > results.len()-1 { break; }
  }

  Ok(())
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
#[derive(Derivative)]
#[derivative(Debug)]
struct CakeStats2 {
  capacity: Nla64,
  memory_limit: Nla32,
  memory_used: Nla32,
  avg_netoff: Nla32,
  max_netlen: Nla32,
  max_adjlen: Nla32,
  min_netlen: Nla32,
  min_adjlen: Nla32,
  #[derivative(Debug="ignore")]
  padding: [u8; 20],
  tins: [CakeTin; 4],
}

unsafe impl bytemuck::Zeroable for CakeStats2 {}
unsafe impl bytemuck::Pod for CakeStats2 {}

#[repr(C, packed)]
#[derive(Copy, Clone)]
#[derive(Derivative)]
#[derivative(Debug)]
struct CakeTin {
  threshold_rate64: Nla64,
  sent_bytes64: Nla64,
  backlog_bytes: Nla32,
  target_us: Nla32,
  interval_us: Nla32,
  sent_packets: Nla32,
  dropped_packets: Nla32,
  ecn_marked_packets: Nla32,
  acks_dropped_packets: Nla32,
  peak_delay_us: Nla32,
  avg_delay_us: Nla32,
  base_delay_us: Nla32,
  way_indirect_hits: Nla32,
  way_missed: Nla32,
  way_collisions: Nla32,
  sparse_flows: Nla32,
  bulk_flows: Nla32,
  unresponsive_flows: Nla32,
  max_skblen: Nla32,
  flow_quantum: Nla32,  
}

unsafe impl bytemuck::Zeroable for CakeTin {}
unsafe impl bytemuck::Pod for CakeTin {}

#[repr(C, packed)]
#[derive(Copy, Clone)]
#[derive(Derivative)]
#[derivative(Debug)]
struct Nla64 {
  #[derivative(Debug="ignore")]
  length: u16,
  #[derivative(Debug="ignore")]
  nla_type: u16,
  value: u64,
}

unsafe impl bytemuck::Zeroable for Nla64 {}
unsafe impl bytemuck::Pod for Nla64 {}

#[repr(C, packed)]
#[derive(Copy, Clone)]
#[derive(Derivative)]
#[derivative(Debug)]
struct Nla32 {
  #[derivative(Debug="ignore")]
  length: u16,
  #[derivative(Debug="ignore")]
  nla_type: u16,
  value: u32,
}

unsafe impl bytemuck::Zeroable for Nla32 {}
unsafe impl bytemuck::Pod for Nla32 {}


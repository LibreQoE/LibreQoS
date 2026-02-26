ways_collisions, if you are seeing a lot of these, it's indicative of a ping flood of some kind.
  thresh          6Mbit
  target          5.0ms
  interval      100.0ms
these are the bandwidth and codel settiings. On a vpn interface, or in a place where the typical rtt is to sydney and back, you can set rtt lower in cake, otherwise, don't touch.
 pk_delay        440us
  av_delay         13us
  sp_delay          2us
All these are ewmas (I forget the interval). pk_delay would be a good statistic to graph, tho I suspect that the highest peak delays would be on the lowest end plans, so peak delay by plan would pull out folk that were above average here.
av_delay is pretty useless, like all averages, IMHO. Still I could be wrong.
sp_delay = sparse delay and is an indicator of overload. Customers with the largest sparse delay would be interesting. Ideally this number never cracks 2ms.
backlog - a persistent backlog is OK, but a sign that someone is using a torrent-like app, doing a big backup, or underprovisioned in some way. Seeing a persistent backlog in an AP on the other hand says the AP is overloaded.
pkts
bytes
do what they say and are a summary statistic since invokation of cake. 
way_inds
way_mis
way_cols are indicators of how well the FQ is working, but not ver good ones. 
drops
marks
ack_drop
in general I care about pkts, ack_drops, drops and marks, most, these are best plotted together on an invlog scale. When last I looked robert was combining drops and marks to treat as drops (which they are), but I am caring about the rise of ECN in general. As for the top 10 approach, the largest number of drops or marks relative to packets, would be useful.
  sp_flows            1
  bk_flows            1
  un_flows            0
bulk_flows indicate large transfers. They have nothing to do with diffserv marks, they are just bulk. 
max_len - if this is greater than your MTU, GRO and GSO are loose on your network. 
Quantum varies as a function of bandwidth, it may be 300 is more optimal than an MTU even at higher bandwidths than we first tested cake at.
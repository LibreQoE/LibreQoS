#pragma once

#define SKB_OVERFLOW(start, end, T) ((void *)start + sizeof(struct T) > end)
#define SKB_OVERFLOW_OFFSET(start, end, offset, T) (start + offset + sizeof(struct T) > end)
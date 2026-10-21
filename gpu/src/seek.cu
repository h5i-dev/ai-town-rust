// One greedy seek step. Same control flow as kernel::batch::seek_one:
// move on x, then on y, then reject the whole step if the tile is blocked
// or any other agent (at its original position) is inside the collision radius.
// COLLISION_SQ is 750*750. A tile is 1000 milli-tiles.

#include <stdint.h>

__device__ uint32_t sat_add(uint32_t a, uint32_t b) {
    unsigned long long s = (unsigned long long)a + (unsigned long long)b;
    if (s > 0xffffffffull) {
        return 0xffffffffu;
    }
    return (uint32_t)s;
}

__device__ uint32_t sat_sub(uint32_t a, uint32_t b) {
    if (b > a) {
        return 0u;
    }
    return a - b;
}

__device__ uint32_t sat_mul(uint32_t a, uint32_t b) {
    unsigned long long s = (unsigned long long)a * (unsigned long long)b;
    if (s > 0xffffffffull) {
        return 0xffffffffu;
    }
    return (uint32_t)s;
}

__device__ unsigned long long abs_diff(uint32_t a, uint32_t b) {
    if (a >= b) {
        return (unsigned long long)(a - b);
    }
    return (unsigned long long)(b - a);
}

__device__ unsigned long long dist_sq(uint32_t ax, uint32_t ay, uint32_t bx, uint32_t by) {
    unsigned long long dx = abs_diff(ax, bx);
    unsigned long long dy = abs_diff(ay, by);
    if (dx > 2000000ull || dy > 2000000ull) {
        return 0xffffffffffffffffull;
    }
    return dx * dx + dy * dy;
}

__device__ int mask_blocked(const uint8_t *mask, uint32_t mask_len, uint32_t width, uint32_t height, uint32_t x, uint32_t y) {
    if (width == 0 || height == 0) {
        return 1;
    }
    uint32_t tx = x / 1000u;
    uint32_t ty = y / 1000u;
    if (tx >= width || ty >= height) {
        return 1;
    }
    uint32_t i = tx * height + ty;
    uint32_t byte = i / 8u;
    uint32_t bit = i % 8u;
    if (byte >= mask_len) {
        return 1;
    }
    return ((mask[byte] >> bit) & 1u) == 1u;
}

__device__ int crowded(const uint32_t *xs, const uint32_t *ys, uint32_t n, uint32_t x, uint32_t y, uint32_t self_i) {
    uint32_t j = 0;
    while (j < n) {
        if (j != self_i && dist_sq(xs[j], ys[j], x, y) < 562500ull) {
            return 1;
        }
        j += 1;
    }
    return 0;
}

extern "C" __global__ void seek_kernel(
    const uint32_t *xs,
    const uint32_t *ys,
    const uint32_t *txs,
    const uint32_t *tys,
    uint32_t *out_x,
    uint32_t *out_y,
    const uint8_t *mask,
    uint32_t mask_len,
    uint32_t n,
    uint32_t step,
    uint32_t width,
    uint32_t height
) {
    uint32_t i = (uint32_t)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i >= n) {
        return;
    }
    uint32_t x = xs[i];
    uint32_t y = ys[i];
    uint32_t goal_x = sat_mul(txs[i], 1000u);
    uint32_t goal_y = sat_mul(tys[i], 1000u);
    uint32_t nx = x;
    uint32_t ny = y;
    uint32_t left = step;
    if (nx != goal_x) {
        uint32_t room = (uint32_t)abs_diff(nx, goal_x);
        uint32_t hop = room < left ? room : left;
        if (goal_x > nx) {
            nx = sat_add(nx, hop);
        } else {
            nx = sat_sub(nx, hop);
        }
        left -= hop;
    }
    if (left > 0u && ny != goal_y) {
        uint32_t room = (uint32_t)abs_diff(ny, goal_y);
        uint32_t hop = room < left ? room : left;
        if (goal_y > ny) {
            ny = sat_add(ny, hop);
        } else {
            ny = sat_sub(ny, hop);
        }
    }
    if (mask_blocked(mask, mask_len, width, height, nx, ny) || crowded(xs, ys, n, nx, ny, i)) {
        out_x[i] = x;
        out_y[i] = y;
    } else {
        out_x[i] = nx;
        out_y[i] = ny;
    }
}

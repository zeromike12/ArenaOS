//! Deterministic 3D vector and matrix mathematics for software rasterization.
//!
//! Fully portable without standard library floating-point hardware dependencies,
//! providing bit-identical results across host tests and bare-metal environments.

#![allow(dead_code)]

pub const PI: f32 = 3.14159265358979323846;
pub const TWO_PI: f32 = 6.28318530717958647692;
pub const HALF_PI: f32 = 1.57079632679489661923;

/// Deterministic floor function for integer conversion without std/libm.
pub fn det_floor(x: f32) -> f32 {
    let i = x as i32;
    if x < (i as f32) {
        (i - 1) as f32
    } else {
        i as f32
    }
}

/// Deterministic ceiling function for integer conversion without std/libm.
pub fn det_ceil(x: f32) -> f32 {
    let i = x as i32;
    if x > (i as f32) {
        (i + 1) as f32
    } else {
        i as f32
    }
}

/// Deterministic sine using high-order polynomial approximation.
pub fn det_sin(mut x: f32) -> f32 {
    // Range reduction to [-PI, PI]
    let quotient = (x / TWO_PI) as i32;
    x -= (quotient as f32) * TWO_PI;
    if x > PI {
        x -= TWO_PI;
    } else if x < -PI {
        x += TWO_PI;
    }

    // Secondary reduction to [-PI/2, PI/2]
    if x > HALF_PI {
        x = PI - x;
    } else if x < -HALF_PI {
        x = -PI - x;
    }

    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;

    // Polynomial approximation of sin(x)
    x - (x3 / 6.0) + (x5 / 120.0) - (x7 / 5040.0) + (x9 / 362880.0)
}

/// Deterministic cosine via phase shift.
pub fn det_cos(x: f32) -> f32 {
    det_sin(x + HALF_PI)
}

/// Deterministic square root using Newton-Raphson iterations.
pub fn det_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    // Initial guess
    let mut guess = if x >= 1.0 { x * 0.5 } else { 1.0 };
    for _ in 0..10 {
        guess = 0.5 * (guess + x / guess);
    }
    guess
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    pub fn scale(self, s: f32) -> Self {
        Self {
            x: self.x * s,
            y: self.y * s,
        }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }

    pub fn scale(self, s: f32) -> Self {
        Self {
            x: self.x * s,
            y: self.y * s,
            z: self.z * s,
        }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        det_sqrt(self.length_squared())
    }

    pub fn normalize(self) -> Self {
        let len = self.length();
        if len > 1e-6 {
            self.scale(1.0 / len)
        } else {
            Self::ZERO
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub const fn from_vec3(v: Vec3, w: f32) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
            w,
        }
    }

    pub fn to_vec3(self) -> Vec3 {
        Vec3 {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }
}

/// 4x4 matrix stored in column-major order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub m: [f32; 16],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        m: [
            1.0, 0.0, 0.0, 0.0, // col 0
            0.0, 1.0, 0.0, 0.0, // col 1
            0.0, 0.0, 1.0, 0.0, // col 2
            0.0, 0.0, 0.0, 1.0, // col 3
        ],
    };

    pub const fn zero() -> Self {
        Self { m: [0.0; 16] }
    }

    pub fn mul(&self, rhs: &Self) -> Self {
        let mut out = Self::zero();
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.m[k * 4 + row] * rhs.m[col * 4 + k];
                }
                out.m[col * 4 + row] = sum;
            }
        }
        out
    }

    pub fn mul_vec4(&self, v: Vec4) -> Vec4 {
        Vec4 {
            x: self.m[0] * v.x + self.m[4] * v.y + self.m[8] * v.z + self.m[12] * v.w,
            y: self.m[1] * v.x + self.m[5] * v.y + self.m[9] * v.z + self.m[13] * v.w,
            z: self.m[2] * v.x + self.m[6] * v.y + self.m[10] * v.z + self.m[14] * v.w,
            w: self.m[3] * v.x + self.m[7] * v.y + self.m[11] * v.z + self.m[15] * v.w,
        }
    }

    pub fn translation(tx: f32, ty: f32, tz: f32) -> Self {
        let mut res = Self::IDENTITY;
        res.m[12] = tx;
        res.m[13] = ty;
        res.m[14] = tz;
        res
    }

    pub fn scale(sx: f32, sy: f32, sz: f32) -> Self {
        let mut res = Self::zero();
        res.m[0] = sx;
        res.m[5] = sy;
        res.m[10] = sz;
        res.m[15] = 1.0;
        res
    }

    pub fn rotation_x(rad: f32) -> Self {
        let s = det_sin(rad);
        let c = det_cos(rad);
        let mut res = Self::IDENTITY;
        res.m[5] = c;
        res.m[6] = s;
        res.m[9] = -s;
        res.m[10] = c;
        res
    }

    pub fn rotation_y(rad: f32) -> Self {
        let s = det_sin(rad);
        let c = det_cos(rad);
        let mut res = Self::IDENTITY;
        res.m[0] = c;
        res.m[2] = -s;
        res.m[8] = s;
        res.m[10] = c;
        res
    }

    pub fn rotation_z(rad: f32) -> Self {
        let s = det_sin(rad);
        let c = det_cos(rad);
        let mut res = Self::IDENTITY;
        res.m[0] = c;
        res.m[1] = s;
        res.m[4] = -s;
        res.m[5] = c;
        res
    }

    /// Perspective projection matrix (OpenGL-style depth mapping to [-1, 1]).
    pub fn perspective(fov_rad: f32, aspect: f32, near: f32, far: f32) -> Self {
        let tan_half_fov = det_sin(fov_rad * 0.5) / det_cos(fov_rad * 0.5);
        let f = 1.0 / tan_half_fov;
        let mut res = Self::zero();
        res.m[0] = f / aspect;
        res.m[5] = f;
        res.m[10] = -(far + near) / (far - near);
        res.m[11] = -1.0;
        res.m[14] = -(2.0 * far * near) / (far - near);
        res
    }

    /// Standard Look-At camera view matrix.
    pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Self {
        let forward = center.sub(eye).normalize();
        let right = forward.cross(up).normalize();
        let true_up = right.cross(forward);

        let mut res = Self::IDENTITY;
        res.m[0] = right.x;
        res.m[4] = right.y;
        res.m[8] = right.z;
        res.m[12] = -right.dot(eye);

        res.m[1] = true_up.x;
        res.m[5] = true_up.y;
        res.m[9] = true_up.z;
        res.m[13] = -true_up.dot(eye);

        res.m[2] = -forward.x;
        res.m[6] = -forward.y;
        res.m[10] = -forward.z;
        res.m[14] = forward.dot(eye);

        res
    }
}

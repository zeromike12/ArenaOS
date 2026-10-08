//! Tests for deterministic 3D math operations.

use arena_graphics_prototype::math::{
    Mat4, PI, Vec3, Vec4, det_cos, det_sin, det_sqrt,
};

#[test]
fn test_deterministic_trig_and_sqrt() {
    // Exact or near-exact values at key angles
    let sin_0 = det_sin(0.0);
    assert!(sin_0.abs() < 1e-6, "sin(0) must be ~0, got {sin_0}");

    let sin_pi_2 = det_sin(PI * 0.5);
    assert!((sin_pi_2 - 1.0).abs() < 1e-4, "sin(pi/2) must be ~1, got {sin_pi_2}");

    let cos_0 = det_cos(0.0);
    assert!((cos_0 - 1.0).abs() < 1e-4, "cos(0) must be ~1, got {cos_0}");

    let cos_pi = det_cos(PI);
    assert!((cos_pi - (-1.0)).abs() < 1e-4, "cos(pi) must be ~-1, got {cos_pi}");

    // Square root
    assert_eq!(det_sqrt(0.0), 0.0);
    assert!((det_sqrt(4.0) - 2.0).abs() < 1e-5);
    assert!((det_sqrt(9.0) - 3.0).abs() < 1e-5);
    assert!((det_sqrt(2.0) - 1.4142135).abs() < 1e-4);
}

#[test]
fn test_vector_operations() {
    let v1 = Vec3::new(1.0, 2.0, 3.0);
    let v2 = Vec3::new(4.0, 5.0, 6.0);

    assert_eq!(v1.add(v2), Vec3::new(5.0, 7.0, 9.0));
    assert_eq!(v2.sub(v1), Vec3::new(3.0, 3.0, 3.0));
    assert_eq!(v1.scale(2.0), Vec3::new(2.0, 4.0, 6.0));

    // Dot product: 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
    assert_eq!(v1.dot(v2), 32.0);

    // Cross product: (2*6 - 3*5, 3*4 - 1*6, 1*5 - 2*4) = (-3, 6, -3)
    let cross = v1.cross(v2);
    assert_eq!(cross, Vec3::new(-3.0, 6.0, -3.0));
    assert_eq!(cross.dot(v1), 0.0); // Orthogonal to v1
    assert_eq!(cross.dot(v2), 0.0); // Orthogonal to v2

    // Normalization
    let v3 = Vec3::new(0.0, 3.0, 4.0);
    assert!((v3.length() - 5.0).abs() < 1e-4);
    let norm = v3.normalize();
    assert!((norm.length() - 1.0).abs() < 1e-4);
}

#[test]
fn test_matrix_transforms() {
    let ident = Mat4::IDENTITY;
    let v = Vec4::new(1.0, 2.0, 3.0, 1.0);
    assert_eq!(ident.mul_vec4(v), v);

    // Translation
    let trans = Mat4::translation(5.0, -3.0, 2.0);
    let vt = trans.mul_vec4(v);
    assert_eq!(vt, Vec4::new(6.0, -1.0, 5.0, 1.0));

    // Scale
    let scale = Mat4::scale(2.0, 3.0, 4.0);
    let vs = scale.mul_vec4(v);
    assert_eq!(vs, Vec4::new(2.0, 6.0, 12.0, 1.0));

    // Combined Scale then Translate
    let combined = trans.mul(&scale);
    let vc = combined.mul_vec4(v);
    assert_eq!(vc, Vec4::new(2.0 + 5.0, 6.0 - 3.0, 12.0 + 2.0, 1.0));
}

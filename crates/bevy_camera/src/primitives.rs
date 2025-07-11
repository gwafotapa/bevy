use core::borrow::Borrow;

use bevy_ecs::{component::Component, entity::EntityHashMap, reflect::ReflectComponent};
use bevy_math::{Affine3A, Mat3A, Mat4, Vec3, Vec3A, Vec4, Vec4Swizzles};
use bevy_mesh::{Mesh, VertexAttributeValues};
use bevy_reflect::prelude::*;

pub trait MeshAabb {
    /// Compute the Axis-Aligned Bounding Box of the mesh vertices in model space
    ///
    /// Returns `None` if `self` doesn't have [`Mesh::ATTRIBUTE_POSITION`] of
    /// type [`VertexAttributeValues::Float32x3`], or if `self` doesn't have any vertices.
    fn compute_aabb(&self) -> Option<Aabb>;
}

impl MeshAabb for Mesh {
    fn compute_aabb(&self) -> Option<Aabb> {
        let Some(VertexAttributeValues::Float32x3(values)) =
            self.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return None;
        };

        Aabb::enclosing(values.iter().map(|p| Vec3::from_slice(p)))
    }
}

/// An axis-aligned bounding box, defined by:
/// - a center,
/// - the distances from the center to each faces along the axis,
///   the faces are orthogonal to the axis.
///
/// It is typically used as a component on an entity to represent the local space
/// occupied by this entity, with faces orthogonal to its local axis.
///
/// This component is notably used during "frustum culling", a process to determine
/// if an entity should be rendered by a [`Camera`] if its bounding box intersects
/// with the camera's [`Frustum`].
///
/// It will be added automatically by the systems in [`CalculateBounds`] to entities that:
/// - could be subject to frustum culling, for example with a [`Mesh3d`]
///   or `Sprite` component,
/// - don't have the [`NoFrustumCulling`] component.
///
/// It won't be updated automatically if the space occupied by the entity changes,
/// for example if the vertex positions of a [`Mesh3d`] are updated.
///
/// [`Camera`]: crate::Camera
/// [`NoFrustumCulling`]: crate::visibility::NoFrustumCulling
/// [`CalculateBounds`]: crate::visibility::VisibilitySystems::CalculateBounds
/// [`Mesh3d`]: bevy_mesh::Mesh
#[derive(Component, Clone, Copy, Debug, Default, Reflect, PartialEq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
pub struct Aabb {
    pub center: Vec3A,
    pub half_extents: Vec3A,
}

impl Aabb {
    #[inline]
    pub fn from_min_max(minimum: Vec3, maximum: Vec3) -> Self {
        let minimum = Vec3A::from(minimum);
        let maximum = Vec3A::from(maximum);
        let center = 0.5 * (maximum + minimum);
        let half_extents = 0.5 * (maximum - minimum);
        Self {
            center,
            half_extents,
        }
    }

    /// Returns a bounding box enclosing the specified set of points.
    ///
    /// Returns `None` if the iterator is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use bevy_math::{Vec3, Vec3A};
    /// # use bevy_camera::primitives::Aabb;
    /// let bb = Aabb::enclosing([Vec3::X, Vec3::Z * 2.0, Vec3::Y * -0.5]).unwrap();
    /// assert_eq!(bb.min(), Vec3A::new(0.0, -0.5, 0.0));
    /// assert_eq!(bb.max(), Vec3A::new(1.0, 0.0, 2.0));
    /// ```
    pub fn enclosing<T: Borrow<Vec3>>(iter: impl IntoIterator<Item = T>) -> Option<Self> {
        let mut iter = iter.into_iter().map(|p| *p.borrow());
        let mut min = iter.next()?;
        let mut max = min;
        for v in iter {
            min = Vec3::min(min, v);
            max = Vec3::max(max, v);
        }
        Some(Self::from_min_max(min, max))
    }

    /// Calculate the relative radius of the AABB with respect to a plane
    #[inline]
    pub fn relative_radius(&self, p_normal: &Vec3A, world_from_local: &Mat3A) -> f32 {
        // NOTE: dot products on Vec3A use SIMD and even with the overhead of conversion are net faster than Vec3
        let half_extents = self.half_extents;
        Vec3A::new(
            p_normal.dot(world_from_local.x_axis),
            p_normal.dot(world_from_local.y_axis),
            p_normal.dot(world_from_local.z_axis),
        )
        .abs()
        .dot(half_extents)
    }

    #[inline]
    pub fn min(&self) -> Vec3A {
        self.center - self.half_extents
    }

    #[inline]
    pub fn max(&self) -> Vec3A {
        self.center + self.half_extents
    }

    /// Check if the AABB is at the front side of the bisecting plane.
    /// Referenced from: [AABB Plane intersection](https://gdbooks.gitbooks.io/3dcollisions/content/Chapter2/static_aabb_plane.html)
    #[inline]
    pub fn is_in_half_space(&self, half_space: &HalfSpace, world_from_local: &Affine3A) -> bool {
        // transform the half-extents into world space.
        let half_extents_world = world_from_local.matrix3.abs() * self.half_extents.abs();
        // collapse the half-extents onto the plane normal.
        let p_normal = half_space.normal();
        let r = half_extents_world.dot(p_normal.abs());
        let aabb_center_world = world_from_local.transform_point3a(self.center);
        let signed_distance = p_normal.dot(aabb_center_world) + half_space.d();
        signed_distance > r
    }
}

impl From<Sphere> for Aabb {
    #[inline]
    fn from(sphere: Sphere) -> Self {
        Self {
            center: sphere.center,
            half_extents: Vec3A::splat(sphere.radius),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sphere {
    pub center: Vec3A,
    pub radius: f32,
}

impl Sphere {
    #[inline]
    pub fn intersects_obb(&self, aabb: &Aabb, world_from_local: &Affine3A) -> bool {
        let aabb_center_world = world_from_local.transform_point3a(aabb.center);
        let v = aabb_center_world - self.center;
        let d = v.length();
        let relative_radius = aabb.relative_radius(&(v / d), &world_from_local.matrix3);
        d < self.radius + relative_radius
    }
}

/// A region of 3D space, specifically an open set whose border is a bisecting 2D plane.
///
/// This bisecting plane partitions 3D space into two infinite regions,
/// the half-space is one of those regions and excludes the bisecting plane.
///
/// Each instance of this type is characterized by:
/// - the bisecting plane's unit normal, normalized and pointing "inside" the half-space,
/// - the signed distance along the normal from the bisecting plane to the origin of 3D space.
///
/// The distance can also be seen as:
/// - the distance along the inverse of the normal from the origin of 3D space to the bisecting plane,
/// - the opposite of the distance along the normal from the origin of 3D space to the bisecting plane.
///
/// Any point `p` is considered to be within the `HalfSpace` when the length of the projection
/// of p on the normal is greater or equal than the opposite of the distance,
/// meaning: if the equation `normal.dot(p) + distance > 0.` is satisfied.
///
/// For example, the half-space containing all the points with a z-coordinate lesser
/// or equal than `8.0` would be defined by: `HalfSpace::new(Vec3::NEG_Z.extend(-8.0))`.
/// It includes all the points from the bisecting plane towards `NEG_Z`, and the distance
/// from the plane to the origin is `-8.0` along `NEG_Z`.
///
/// It is used to define a [`Frustum`], but is also a useful mathematical primitive for rendering tasks such as  light computation.
#[derive(Clone, Copy, Debug, Default)]
pub struct HalfSpace {
    normal_d: Vec4,
}

impl HalfSpace {
    /// Constructs a `HalfSpace` from a 4D vector whose first 3 components
    /// represent the bisecting plane's unit normal, and the last component is
    /// the signed distance along the normal from the plane to the origin.
    /// The constructor ensures the normal vector is normalized and the distance is appropriately scaled.
    #[inline]
    pub fn new(normal_d: Vec4) -> Self {
        Self {
            normal_d: normal_d * normal_d.xyz().length_recip(),
        }
    }

    /// Returns the unit normal vector of the bisecting plane that characterizes the `HalfSpace`.
    #[inline]
    pub fn normal(&self) -> Vec3A {
        Vec3A::from_vec4(self.normal_d)
    }

    /// Returns the signed distance from the bisecting plane to the origin along
    /// the plane's unit normal vector.
    #[inline]
    pub fn d(&self) -> f32 {
        self.normal_d.w
    }

    /// Returns the bisecting plane's unit normal vector and the signed distance
    /// from the plane to the origin.
    #[inline]
    pub fn normal_d(&self) -> Vec4 {
        self.normal_d
    }
}

/// A region of 3D space defined by the intersection of 6 [`HalfSpace`]s.
///
/// Frustums are typically an apex-truncated square pyramid (a pyramid without the top) or a cuboid.
///
/// Half spaces are ordered left, right, top, bottom, near, far. The normal vectors
/// of the half-spaces point towards the interior of the frustum.
///
/// A frustum component is used on an entity with a [`Camera`] component to
/// determine which entities will be considered for rendering by this camera.
/// All entities with an [`Aabb`] component that are not contained by (or crossing
/// the boundary of) the frustum will not be rendered, and not be used in rendering computations.
///
/// This process is called frustum culling, and entities can opt out of it using
/// the [`NoFrustumCulling`] component.
///
/// The frustum component is typically added automatically for cameras, either `Camera2d` or `Camera3d`.
/// It is usually updated automatically by [`update_frusta`] from the
/// [`CameraProjection`] component and [`GlobalTransform`] of the camera entity.
///
/// [`Camera`]: crate::Camera
/// [`NoFrustumCulling`]: crate::visibility::NoFrustumCulling
/// [`update_frusta`]: crate::visibility::update_frusta
/// [`CameraProjection`]: crate::CameraProjection
/// [`GlobalTransform`]: bevy_transform::components::GlobalTransform
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default, Debug, Clone)]
pub struct Frustum {
    #[reflect(ignore, clone)]
    pub half_spaces: [HalfSpace; 6],
}

impl Frustum {
    /// Returns a frustum derived from `clip_from_world`.
    #[inline]
    pub fn from_clip_from_world(clip_from_world: &Mat4) -> Self {
        let mut frustum = Frustum::from_clip_from_world_no_far(clip_from_world);
        frustum.half_spaces[5] = HalfSpace::new(clip_from_world.row(2));
        frustum
    }

    /// Returns a frustum derived from `clip_from_world`,
    /// but with a custom far plane.
    #[inline]
    pub fn from_clip_from_world_custom_far(
        clip_from_world: &Mat4,
        view_translation: &Vec3,
        view_backward: &Vec3,
        far: f32,
    ) -> Self {
        let mut frustum = Frustum::from_clip_from_world_no_far(clip_from_world);
        let far_center = *view_translation - far * *view_backward;
        frustum.half_spaces[5] =
            HalfSpace::new(view_backward.extend(-view_backward.dot(far_center)));
        frustum
    }

    // NOTE: This approach of extracting the frustum half-space from the view
    // projection matrix is from Foundations of Game Engine Development 2
    // Rendering by Lengyel.
    /// Returns a frustum derived from `view_projection`,
    /// without a far plane.
    fn from_clip_from_world_no_far(clip_from_world: &Mat4) -> Self {
        let row3 = clip_from_world.row(3);
        let mut half_spaces = [HalfSpace::default(); 6];
        for (i, half_space) in half_spaces.iter_mut().enumerate().take(5) {
            let row = clip_from_world.row(i / 2);
            *half_space = HalfSpace::new(if (i & 1) == 0 && i != 4 {
                row3 + row
            } else {
                row3 - row
            });
        }
        Self { half_spaces }
    }

    /// Checks if a sphere intersects the frustum.
    #[inline]
    pub fn intersects_sphere(&self, sphere: &Sphere, intersect_far: bool) -> bool {
        let sphere_center = sphere.center.extend(1.0);
        let max = if intersect_far { 6 } else { 5 };
        for half_space in &self.half_spaces[..max] {
            if half_space.normal_d().dot(sphere_center) + sphere.radius <= 0.0 {
                return false;
            }
        }
        true
    }

    /// Checks if an Oriented Bounding Box (obb) intersects the frustum.
    #[inline]
    pub fn intersects_obb(
        &self,
        aabb: &Aabb,
        world_from_local: &Affine3A,
        intersect_near: bool,
        intersect_far: bool,
    ) -> bool {
        let aabb_center_world = world_from_local.transform_point3a(aabb.center).extend(1.0);
        for (idx, half_space) in self.half_spaces.into_iter().enumerate() {
            if idx == 4 && !intersect_near {
                continue;
            }
            if idx == 5 && !intersect_far {
                continue;
            }
            let p_normal = half_space.normal();
            let relative_radius = aabb.relative_radius(&p_normal, &world_from_local.matrix3);
            if half_space.normal_d().dot(aabb_center_world) + relative_radius <= 0.0 {
                return false;
            }
        }
        true
    }

    /// Check if the frustum contains the Axis-Aligned Bounding Box (AABB).
    /// Referenced from: [Frustum Culling](https://learnopengl.com/Guest-Articles/2021/Scene/Frustum-Culling)
    #[inline]
    pub fn contains_aabb(&self, aabb: &Aabb, world_from_local: &Affine3A) -> bool {
        for half_space in &self.half_spaces {
            if !aabb.is_in_half_space(half_space, world_from_local) {
                return false;
            }
        }
        true
    }
}

#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component, Default, Debug, Clone)]
pub struct CubemapFrusta {
    #[reflect(ignore, clone)]
    pub frusta: [Frustum; 6],
}

impl CubemapFrusta {
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Frustum> {
        self.frusta.iter()
    }
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Frustum> {
        self.frusta.iter_mut()
    }
}

#[derive(Component, Debug, Default, Reflect, Clone)]
#[reflect(Component, Default, Debug, Clone)]
pub struct CascadesFrusta {
    #[reflect(ignore, clone)]
    pub frusta: EntityHashMap<Vec<Frustum>>,
}

#[cfg(test)]
mod tests {
    use core::f32::consts::PI;

    use bevy_math::{ops, Quat};
    use bevy_transform::components::GlobalTransform;

    use crate::{CameraProjection, PerspectiveProjection};

    use super::*;

    // A big, offset frustum
    fn big_frustum() -> Frustum {
        Frustum {
            half_spaces: [
                HalfSpace::new(Vec4::new(-0.9701, -0.2425, -0.0000, 7.7611)),
                HalfSpace::new(Vec4::new(-0.0000, 1.0000, -0.0000, 4.0000)),
                HalfSpace::new(Vec4::new(-0.0000, -0.2425, -0.9701, 2.9104)),
                HalfSpace::new(Vec4::new(-0.0000, -1.0000, -0.0000, 4.0000)),
                HalfSpace::new(Vec4::new(-0.0000, -0.2425, 0.9701, 2.9104)),
                HalfSpace::new(Vec4::new(0.9701, -0.2425, -0.0000, -1.9403)),
            ],
        }
    }

    #[test]
    fn intersects_sphere_big_frustum_outside() {
        // Sphere outside frustum
        let frustum = big_frustum();
        let sphere = Sphere {
            center: Vec3A::new(0.9167, 0.0000, 0.0000),
            radius: 0.7500,
        };
        assert!(!frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_big_frustum_intersect() {
        // Sphere intersects frustum boundary
        let frustum = big_frustum();
        let sphere = Sphere {
            center: Vec3A::new(7.9288, 0.0000, 2.9728),
            radius: 2.0000,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    // A frustum
    fn frustum() -> Frustum {
        Frustum {
            half_spaces: [
                HalfSpace::new(Vec4::new(-0.9701, -0.2425, -0.0000, 0.7276)),
                HalfSpace::new(Vec4::new(-0.0000, 1.0000, -0.0000, 1.0000)),
                HalfSpace::new(Vec4::new(-0.0000, -0.2425, -0.9701, 0.7276)),
                HalfSpace::new(Vec4::new(-0.0000, -1.0000, -0.0000, 1.0000)),
                HalfSpace::new(Vec4::new(-0.0000, -0.2425, 0.9701, 0.7276)),
                HalfSpace::new(Vec4::new(0.9701, -0.2425, -0.0000, 0.7276)),
            ],
        }
    }

    #[test]
    fn intersects_sphere_frustum_surrounding() {
        // Sphere surrounds frustum
        let frustum = frustum();
        let sphere = Sphere {
            center: Vec3A::new(0.0000, 0.0000, 0.0000),
            radius: 3.0000,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_frustum_contained() {
        // Sphere is contained in frustum
        let frustum = frustum();
        let sphere = Sphere {
            center: Vec3A::new(0.0000, 0.0000, 0.0000),
            radius: 0.7000,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_frustum_intersects_plane() {
        // Sphere intersects a plane
        let frustum = frustum();
        let sphere = Sphere {
            center: Vec3A::new(0.0000, 0.0000, 0.9695),
            radius: 0.7000,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_frustum_intersects_2_planes() {
        // Sphere intersects 2 planes
        let frustum = frustum();
        let sphere = Sphere {
            center: Vec3A::new(1.2037, 0.0000, 0.9695),
            radius: 0.7000,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_frustum_intersects_3_planes() {
        // Sphere intersects 3 planes
        let frustum = frustum();
        let sphere = Sphere {
            center: Vec3A::new(1.2037, -1.0988, 0.9695),
            radius: 0.7000,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_frustum_dodges_1_plane() {
        // Sphere avoids intersecting the frustum by 1 plane
        let frustum = frustum();
        let sphere = Sphere {
            center: Vec3A::new(-1.7020, 0.0000, 0.0000),
            radius: 0.7000,
        };
        assert!(!frustum.intersects_sphere(&sphere, true));
    }

    // A long frustum.
    fn long_frustum() -> Frustum {
        Frustum {
            half_spaces: [
                HalfSpace::new(Vec4::new(-0.9998, -0.0222, -0.0000, -1.9543)),
                HalfSpace::new(Vec4::new(-0.0000, 1.0000, -0.0000, 45.1249)),
                HalfSpace::new(Vec4::new(-0.0000, -0.0168, -0.9999, 2.2718)),
                HalfSpace::new(Vec4::new(-0.0000, -1.0000, -0.0000, 45.1249)),
                HalfSpace::new(Vec4::new(-0.0000, -0.0168, 0.9999, 2.2718)),
                HalfSpace::new(Vec4::new(0.9998, -0.0222, -0.0000, 7.9528)),
            ],
        }
    }

    #[test]
    fn intersects_sphere_long_frustum_outside() {
        // Sphere outside frustum
        let frustum = long_frustum();
        let sphere = Sphere {
            center: Vec3A::new(-4.4889, 46.9021, 0.0000),
            radius: 0.7500,
        };
        assert!(!frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn intersects_sphere_long_frustum_intersect() {
        // Sphere intersects frustum boundary
        let frustum = long_frustum();
        let sphere = Sphere {
            center: Vec3A::new(-4.9957, 0.0000, -0.7396),
            radius: 4.4094,
        };
        assert!(frustum.intersects_sphere(&sphere, true));
    }

    #[test]
    fn aabb_enclosing() {
        assert_eq!(Aabb::enclosing(<[Vec3; 0]>::default()), None);
        assert_eq!(
            Aabb::enclosing(vec![Vec3::ONE]).unwrap(),
            Aabb::from_min_max(Vec3::ONE, Vec3::ONE)
        );
        assert_eq!(
            Aabb::enclosing(&[Vec3::Y, Vec3::X, Vec3::Z][..]).unwrap(),
            Aabb::from_min_max(Vec3::ZERO, Vec3::ONE)
        );
        assert_eq!(
            Aabb::enclosing([
                Vec3::NEG_X,
                Vec3::X * 2.0,
                Vec3::NEG_Y * 5.0,
                Vec3::Z,
                Vec3::ZERO
            ])
            .unwrap(),
            Aabb::from_min_max(Vec3::new(-1.0, -5.0, 0.0), Vec3::new(2.0, 0.0, 1.0))
        );
    }

    // A frustum with an offset for testing the [`Frustum::contains_aabb`] algorithm.
    fn contains_aabb_test_frustum() -> Frustum {
        let proj = PerspectiveProjection {
            fov: 90.0_f32.to_radians(),
            aspect_ratio: 1.0,
            near: 1.0,
            far: 100.0,
        };
        proj.compute_frustum(&GlobalTransform::from_translation(Vec3::new(2.0, 2.0, 0.0)))
    }

    fn contains_aabb_test_frustum_with_rotation() -> Frustum {
        let half_extent_world = (((49.5 * 49.5) * 0.5) as f32).sqrt() + 0.5f32.sqrt();
        let near = 50.5 - half_extent_world;
        let far = near + 2.0 * half_extent_world;
        let fov = 2.0 * ops::atan(half_extent_world / near);
        let proj = PerspectiveProjection {
            aspect_ratio: 1.0,
            near,
            far,
            fov,
        };
        proj.compute_frustum(&GlobalTransform::IDENTITY)
    }

    #[test]
    fn aabb_inside_frustum() {
        let frustum = contains_aabb_test_frustum();
        let aabb = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::new(0.99, 0.99, 49.49),
        };
        let model = Affine3A::from_translation(Vec3::new(2.0, 2.0, -50.5));
        assert!(frustum.contains_aabb(&aabb, &model));
    }

    #[test]
    fn aabb_intersect_frustum() {
        let frustum = contains_aabb_test_frustum();
        let aabb = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::new(0.99, 0.99, 49.6),
        };
        let model = Affine3A::from_translation(Vec3::new(2.0, 2.0, -50.5));
        assert!(!frustum.contains_aabb(&aabb, &model));
    }

    #[test]
    fn aabb_outside_frustum() {
        let frustum = contains_aabb_test_frustum();
        let aabb = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::new(0.99, 0.99, 0.99),
        };
        let model = Affine3A::from_translation(Vec3::new(0.0, 0.0, 49.6));
        assert!(!frustum.contains_aabb(&aabb, &model));
    }

    #[test]
    fn aabb_inside_frustum_rotation() {
        let frustum = contains_aabb_test_frustum_with_rotation();
        let aabb = Aabb {
            center: Vec3A::new(0.0, 0.0, 0.0),
            half_extents: Vec3A::new(0.99, 0.99, 49.49),
        };

        let model = Affine3A::from_rotation_translation(
            Quat::from_rotation_x(PI / 4.0),
            Vec3::new(0.0, 0.0, -50.5),
        );
        assert!(frustum.contains_aabb(&aabb, &model));
    }

    #[test]
    fn aabb_intersect_frustum_rotation() {
        let frustum = contains_aabb_test_frustum_with_rotation();
        let aabb = Aabb {
            center: Vec3A::new(0.0, 0.0, 0.0),
            half_extents: Vec3A::new(0.99, 0.99, 49.6),
        };

        let model = Affine3A::from_rotation_translation(
            Quat::from_rotation_x(PI / 4.0),
            Vec3::new(0.0, 0.0, -50.5),
        );
        assert!(!frustum.contains_aabb(&aabb, &model));
    }
}

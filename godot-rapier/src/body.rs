#![allow(clippy::option_if_let_else)]
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use godot::{
    engine::{
        physics_server_3d::BodyMode,
        physics_server_3d::{AreaSpaceOverrideMode, BodyParameter, BodyState},
        rigid_body_3d::DampMode,
        PhysicsDirectBodyState3D,
    },
    prelude::*,
};
use rapier3d::prelude::*;

use crate::{
    area::RapierArea,
    collision_object::{Handle, RapierCollisionObject},
    conversions::{
        godot_vector_to_rapier_point, godot_vector_to_rapier_vector, isometry_to_transform,
        rapier_point_to_godot_vector, rapier_vector_to_godot_vector, transform_to_isometry,
    },
    direct_body_state_3d::RapierPhysicsDirectBodyState3D,
    error::RapierError,
    shapes::RapierShapeInstance,
    space::RapierSpace,
};
#[allow(clippy::struct_excessive_bools)]
pub struct RapierBody {
    rid: Rid,
    space: Option<Rc<RefCell<RapierSpace>>>,
    handle: Option<RigidBodyHandle>,
    shapes: Vec<RapierShapeInstance>,
    body_mode: BodyMode,
    instance_id: Option<u64>,
    ccd_enabled: bool,
    body_state_callback: Callable,
    constant_force: Vector<f32>,
    constant_torque: Vector<f32>,

    collision_layer: u32,
    collision_mask: u32,
    collision_priority: f32,

    bounce: f32,
    friction: f32,
    mass: f32,
    inertia: Vector3,
    custom_center_of_mass: Vector3,
    has_custom_center_of_mass: bool,
    gravity_scale: f32,
    linear_damp_mode: DampMode,
    angular_damp_mode: DampMode,
    linear_damp: f32,
    angular_damp: f32,

    areas: Vec<Rc<RefCell<RapierArea>>>,

    linear_velocity: Vector3,
    angular_velocity: Vector3,
    transform: Transform3D,
    kinematic_isometry: Isometry<f32>,
    is_sleeping: bool,
    can_sleep: bool,

    sync_state: bool,
    direct_state: Option<Gd<RapierPhysicsDirectBodyState3D>>,
}

impl Drop for RapierBody {
    fn drop(&mut self) {
        if let Some(direct_state) = self.direct_state.take() {
            direct_state.free();
        }
    }
}

impl RapierCollisionObject for RapierBody {
    fn rid(&self) -> Rid {
        self.rid
    }
    fn set_space(&mut self, space: Rc<RefCell<RapierSpace>>) {
        self.space = Some(space);
    }

    #[track_caller]
    fn space(&self) -> Option<&Rc<RefCell<RapierSpace>>> {
        if self.space.is_none() {
            let caller_location = std::panic::Location::caller();
            let file = caller_location.file();
            let line_number = caller_location.line();
            godot_error!(
                "{} called from {file}:{line_number}",
                RapierError::ObjectSpaceNotSet(self.rid)
            );
        }
        self.space.as_ref()
    }
    fn remove_space(&mut self, remove_from_space: bool) {
        if remove_from_space {
            if let Some(space) = self.space() {
                if let Some(handle) = self.handle() {
                    space.borrow_mut().remove_body(handle);
                }
            }
        }
        self.space = None;
        self.handle = None;
    }

    fn generic_handle(&self) -> Handle {
        self.handle().map_or(Handle::NotSet, Handle::BodyHandle)
    }

    fn shapes(&self) -> &Vec<RapierShapeInstance> {
        &self.shapes
    }

    fn shapes_mut(&mut self) -> &mut Vec<RapierShapeInstance> {
        &mut self.shapes
    }

    fn set_instance_id(&mut self, id: u64) {
        self.instance_id = Some(id);
    }
    fn instance_id(&self) -> Option<u64> {
        if self.instance_id.is_none() {
            godot_error!("{}", RapierError::BodyInstanceIDNotSet(self.rid));
        }
        self.instance_id
    }

    fn isometry(&self) -> Isometry<f32> {
        transform_to_isometry(&self.transform()).0
    }

    fn scale(&self) -> Vector<f32> {
        transform_to_isometry(&self.transform()).1
    }

    fn set_collision_layer(&mut self, layer: u32) {
        self.collision_layer = layer;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_body_collision_group(
                    handle,
                    self.collision_layer,
                    self.collision_mask,
                );
            }
        }
    }

    fn get_collision_layer(&self) -> u32 {
        self.collision_layer
    }

    fn set_collision_mask(&mut self, mask: u32) {
        self.collision_mask = mask;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_body_collision_group(
                    handle,
                    self.collision_layer,
                    self.collision_mask,
                );
            }
        }
    }

    fn get_collision_mask(&self) -> u32 {
        self.collision_mask
    }
}

impl RapierBody {
    pub fn add_constant_central_force(&mut self, force: Vector3) {
        self.constant_force += godot_vector_to_rapier_vector(force);
    }

    pub fn add_constant_force(&mut self, force: Vector3, position: Vector3) {
        let center_of_mass = self.center_of_mass();
        let translation = self.transform().origin;
        let center_of_mass_relative = godot_vector_to_rapier_point(center_of_mass - translation);
        let force = godot_vector_to_rapier_vector(force);
        let position = godot_vector_to_rapier_point(position);
        self.constant_force += force;
        self.constant_torque += (position - center_of_mass_relative).cross(&force);
    }

    pub fn add_constant_torque(&mut self, torque: Vector3) {
        self.constant_torque += godot_vector_to_rapier_vector(torque);
    }

    pub const fn angular_damp(&self) -> f32 {
        self.angular_damp
    }

    pub fn angular_velocity(&self) -> Vector3 {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    return rapier_vector_to_godot_vector(*body.angvel());
                }
            }
        }
        self.angular_velocity
    }

    pub fn apply_central_force(&mut self, force: Vector3) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let force = godot_vector_to_rapier_vector(force);
                space.borrow_mut().apply_central_force(handle, force);
            }
        }
    }

    pub fn apply_central_impulse(&mut self, impulse: Vector3) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let impulse = godot_vector_to_rapier_vector(impulse);
                space.borrow_mut().apply_central_impulse(handle, impulse);
            }
        }
    }

    pub fn apply_force(&mut self, force: Vector3, position: Vector3) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let force = godot_vector_to_rapier_vector(force);
                let position = godot_vector_to_rapier_point(position);
                space.borrow_mut().apply_force(handle, force, position);
            }
        }
    }
    pub fn apply_impulse(&mut self, impulse: Vector3, position: Vector3) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let impulse = godot_vector_to_rapier_vector(impulse);
                let position = godot_vector_to_rapier_point(position);
                space.borrow_mut().apply_impulse(handle, impulse, position);
            }
        }
    }

    pub fn apply_torque(&mut self, torque: Vector3) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let torque = godot_vector_to_rapier_vector(torque);
                space.borrow_mut().apply_torque(handle, torque);
            }
        }
    }

    pub fn apply_torque_impulse(&mut self, impulse: Vector3) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let impulse = godot_vector_to_rapier_vector(impulse);
                space.borrow_mut().apply_torque_impulse(handle, impulse);
            }
        }
    }

    pub const fn body_mode(&self) -> BodyMode {
        self.body_mode
    }
    pub const fn bounce(&self) -> f32 {
        self.bounce
    }

    pub fn call_queries(&mut self) {
        if let Some(direct_state) = &self.direct_state {
            if self.sync_state && self.body_state_callback.is_valid() {
                self.body_state_callback
                    .callv(array![Variant::from(direct_state.share())]);
            }
        }
    }
    pub const fn can_sleep(&self) -> bool {
        self.can_sleep
    }
    pub fn center_of_mass(&self) -> Vector3 {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    return rapier_point_to_godot_vector(*body.center_of_mass());
                }
            }
        }
        Vector3::ZERO
    }
    pub const fn collision_priority(&self) -> f32 {
        self.collision_priority
    }
    pub fn constant_force_godot(&self) -> Vector3 {
        rapier_vector_to_godot_vector(self.constant_force)
    }
    pub fn constant_torque_godot(&self) -> Vector3 {
        rapier_vector_to_godot_vector(self.constant_torque)
    }
    pub const fn custom_center_of_mass(&self) -> Vector3 {
        self.custom_center_of_mass
    }

    pub fn direct_state(&self) -> Option<Gd<PhysicsDirectBodyState3D>> {
        if let Some(direct_state) = &self.direct_state {
            return Some(direct_state.share().upcast::<PhysicsDirectBodyState3D>());
        }
        None
    }

    pub const fn friction(&self) -> f32 {
        self.friction
    }

    pub fn get_param(&self, param: BodyParameter) -> Variant {
        match param {
            BodyParameter::BODY_PARAM_BOUNCE => Variant::from(self.bounce),
            BodyParameter::BODY_PARAM_FRICTION => Variant::from(self.friction),
            BodyParameter::BODY_PARAM_MASS => Variant::from(self.mass),
            BodyParameter::BODY_PARAM_INERTIA => Variant::from(self.inertia),
            BodyParameter::BODY_PARAM_CENTER_OF_MASS => Variant::from(self.center_of_mass()),
            BodyParameter::BODY_PARAM_GRAVITY_SCALE => Variant::from(self.gravity_scale),
            BodyParameter::BODY_PARAM_LINEAR_DAMP_MODE => Variant::from(self.linear_damp_mode),
            BodyParameter::BODY_PARAM_ANGULAR_DAMP_MODE => Variant::from(self.angular_damp_mode),
            BodyParameter::BODY_PARAM_LINEAR_DAMP => Variant::from(self.linear_damp),
            BodyParameter::BODY_PARAM_ANGULAR_DAMP => Variant::from(self.angular_damp),
            _ => Variant::nil(),
        }
    }

    pub fn get_state(&self, state: BodyState) -> Variant {
        match state {
            BodyState::BODY_STATE_TRANSFORM => Variant::from(self.transform()),
            BodyState::BODY_STATE_LINEAR_VELOCITY => Variant::from(self.linear_velocity()),
            BodyState::BODY_STATE_ANGULAR_VELOCITY => Variant::from(self.angular_velocity()),
            BodyState::BODY_STATE_SLEEPING => Variant::from(self.is_sleeping()),
            BodyState::BODY_STATE_CAN_SLEEP => Variant::from(self.can_sleep()),
            _ => Variant::nil(),
        }
    }
    pub const fn gravity_scale(&self) -> f32 {
        self.gravity_scale
    }

    pub fn handle(&self) -> Option<RigidBodyHandle> {
        if self.handle.is_none() {
            godot_error!("{}", RapierError::BodyHandleNotSet(self.rid));
        }
        self.handle
    }
    pub const fn has_custom_center_of_mass(&self) -> bool {
        self.has_custom_center_of_mass
    }

    pub const fn inertia(&self) -> Vector3 {
        self.inertia
    }

    fn integrate_forces(&mut self, step: f32) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let linear_velocity = godot_vector_to_rapier_vector(self.linear_velocity());

                space.borrow_mut().set_linear_velocity(
                    handle,
                    linear_velocity + godot_vector_to_rapier_vector(step * self.total_gravity()),
                );
                space
                    .borrow_mut()
                    .apply_central_force(handle, self.constant_force);
                space
                    .borrow_mut()
                    .apply_torque(handle, self.constant_torque);
            }
        }
    }

    pub fn inverse_inertia(&self) -> Vector3 {
        if self.is_kinematic() || self.is_static() {
            return Vector3::ZERO;
        }
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    let inv_inertia = body
                        .mass_properties()
                        .local_mprops
                        .inv_principal_inertia_sqrt;
                    return Vector3::new(inv_inertia.x, inv_inertia.y, inv_inertia.z);
                }
            }
        }
        self.inertia.inverse()
    }

    pub fn inverse_inertia_tensor(&self) -> Basis {
        if self.is_kinematic() || self.is_static() {
            return Basis::IDENTITY;
        }
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    let inv_inertia = body
                        .mass_properties()
                        .local_mprops
                        .inv_principal_inertia_sqrt;
                    return Basis::from_diagonal(inv_inertia.x, inv_inertia.y, inv_inertia.z);
                }
            }
        }
        let inv = self.inertia.inverse();
        Basis::from_diagonal(inv.x, inv.y, inv.z)
    }

    pub fn inverse_mass(&self) -> f32 {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    return body.mass_properties().local_mprops.inv_mass;
                }
            }
        }
        1.0 / self.mass
    }

    pub const fn is_ccd_enabled(&self) -> bool {
        self.ccd_enabled
    }
    pub fn is_kinematic(&self) -> bool {
        self.body_mode == BodyMode::BODY_MODE_KINEMATIC
    }

    pub fn is_sleeping(&self) -> bool {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    return body.is_sleeping();
                }
            }
        }
        self.is_sleeping
    }

    pub fn is_static(&self) -> bool {
        self.body_mode == BodyMode::BODY_MODE_STATIC
    }

    pub const fn linear_damp(&self) -> f32 {
        self.linear_damp
    }
    pub fn linear_velocity(&self) -> Vector3 {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    return rapier_vector_to_godot_vector(*body.linvel());
                }
            }
        }
        self.linear_velocity
    }
    pub fn local_center_of_mass(&self) -> Vector3 {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    let local_com = body.mass_properties().local_mprops.local_com;
                    return rapier_point_to_godot_vector(local_com);
                }
            }
        }
        Vector3::ZERO
    }
    pub const fn mass(&self) -> f32 {
        self.mass
    }

    fn move_kinematic(&mut self) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space
                    .borrow_mut()
                    .set_linear_velocity(handle, Vector::zeros());
                space
                    .borrow_mut()
                    .set_angular_velocity(handle, Vector::zeros());

                if self.isometry() == self.kinematic_isometry {
                    return;
                }

                space
                    .borrow_mut()
                    .move_kinematic(handle, self.kinematic_isometry);
            }
        }
    }

    pub fn new(rid: Rid) -> Self {
        Self {
            rid,
            space: Option::default(),
            handle: Option::default(),
            shapes: Vec::default(),
            body_mode: BodyMode::BODY_MODE_STATIC,
            instance_id: Option::default(),
            ccd_enabled: Default::default(),
            body_state_callback: Callable::invalid(),
            constant_force: Vector::default(),
            constant_torque: Vector::default(),
            collision_layer: 1,
            collision_mask: 1,
            collision_priority: 1.0,
            bounce: Default::default(),
            friction: Default::default(),
            mass: 1.0,
            inertia: Vector3::default(),
            custom_center_of_mass: Vector3::default(),
            has_custom_center_of_mass: false,
            gravity_scale: 1.0,
            linear_damp_mode: DampMode::DAMP_MODE_COMBINE,
            angular_damp_mode: DampMode::DAMP_MODE_COMBINE,
            linear_damp: Default::default(),
            angular_damp: Default::default(),
            areas: Vec::default(),
            linear_velocity: Vector3::default(),
            angular_velocity: Vector3::default(),
            transform: Transform3D::IDENTITY,
            kinematic_isometry: Isometry::default(),
            is_sleeping: false,
            can_sleep: true,
            sync_state: false,
            direct_state: None,
        }
    }

    pub fn pre_step(&mut self, step: f32) {
        match self.body_mode {
            BodyMode::BODY_MODE_RIGID | BodyMode::BODY_MODE_RIGID_LINEAR => {
                self.integrate_forces(step);
            }
            BodyMode::BODY_MODE_KINEMATIC => {
                self.move_kinematic();
            }
            _ => {}
        };
    }
    pub fn principal_inertia_axes(&self) -> Basis {
        if self.is_kinematic() || self.is_static() {
            return Basis::IDENTITY;
        }

        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    let inertia = body.mass_properties().local_mprops.principal_inertia();
                    return self.transform().basis
                        * Basis::from_diagonal(inertia.x, inertia.y, inertia.z);
                }
            }
        }
        self.transform().basis
            * Basis::from_diagonal(self.inertia.x, self.inertia.y, self.inertia.z)
    }

    pub fn reset_mass_properties(&mut self) {
        self.inertia = Vector3::ZERO;
        self.custom_center_of_mass = Vector3::ZERO;
        self.has_custom_center_of_mass = false;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_mass(handle, self.mass, false);
            }
        }
    }

    pub fn set_angular_damp(&mut self, angular_damp: f32) {
        self.angular_damp = angular_damp;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_angular_damp(handle, angular_damp);
            }
        }
    }

    pub fn set_angular_velocity(&mut self, value: Vector3) {
        self.angular_velocity = value;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let value = godot_vector_to_rapier_vector(value);
                space.borrow_mut().set_angular_velocity(handle, value);
            }
        }
    }

    pub fn set_body_mode(&mut self, mode: BodyMode) {
        self.body_mode = mode;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_body_mode(handle, mode);
                if self.is_kinematic() {
                    self.kinematic_isometry = self.isometry();
                }
            }
        }
    }

    pub fn set_body_state_callback(&mut self, body_state_callback: Callable) {
        self.body_state_callback = body_state_callback;
    }

    pub fn set_bounce(&mut self, bounce: f32) {
        self.bounce = bounce;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_bounce(handle, bounce);
            }
        }
    }

    pub fn set_can_sleep(&mut self, value: bool) {
        self.can_sleep = value;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_can_sleep(handle, value);
            }
        }
    }
    pub fn set_center_of_mass(&mut self, center_of_mass: Vector3) {
        self.custom_center_of_mass = center_of_mass;
        self.has_custom_center_of_mass = true;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space
                    .borrow_mut()
                    .set_custom_center_of_mass(handle, center_of_mass);
            }
        }
    }
    pub fn set_collision_priority(&mut self, priority: f32) {
        self.collision_priority = priority;
    }
    pub fn set_constant_force(&mut self, force: Vector3) {
        self.constant_force = godot_vector_to_rapier_vector(force);
    }

    pub fn set_constant_torque(&mut self, torque: Vector3) {
        self.constant_torque = godot_vector_to_rapier_vector(torque);
    }

    pub fn set_direct_state(&mut self, body: Weak<RefCell<Self>>) {
        let direct_state = RapierPhysicsDirectBodyState3D::new(body);
        self.direct_state = Some(Gd::new(direct_state));
    }

    pub fn set_enable_ccd(&mut self, enabled: bool) {
        self.ccd_enabled = enabled;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_ccd_enabled(handle, enabled);
            }
        }
    }

    pub fn set_friction(&mut self, friction: f32) {
        self.friction = friction;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_friction(handle, friction);
            }
        }
    }

    pub fn set_gravity_scale(&mut self, gravity_scale: f32) {
        self.gravity_scale = gravity_scale;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_gravity_scale(handle, gravity_scale);
            }
        }
    }

    pub fn set_handle(&mut self, handle: RigidBodyHandle) {
        self.handle = Some(handle);
    }

    pub fn set_inertia(&mut self, inertia: Vector3) {
        self.inertia = inertia;

        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if inertia == Vector3::ZERO {
                    space
                        .borrow_mut()
                        .set_mass(handle, self.mass, self.has_custom_center_of_mass);
                } else {
                    space.borrow_mut().set_inertia(handle, inertia);
                }
            }
        }
    }

    pub fn set_is_sleeping(&mut self, value: bool) {
        self.is_sleeping = value;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_is_sleeping(handle, value);
            }
        }
    }

    pub fn set_linear_damp(&mut self, linear_damp: f32) {
        self.angular_damp = linear_damp;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_linear_damp(handle, linear_damp);
            }
        }
    }

    pub fn set_linear_velocity(&mut self, value: Vector3) {
        self.linear_velocity = value;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let value = godot_vector_to_rapier_vector(value);
                space.borrow_mut().set_linear_velocity(handle, value);
            }
        }
    }

    pub fn set_mass(&mut self, mass: f32) {
        self.mass = mass;
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space
                    .borrow_mut()
                    .set_mass(handle, mass, self.has_custom_center_of_mass);
            }
        }
    }

    pub fn set_param(&mut self, param: BodyParameter, value: &Variant) {
        match param {
            BodyParameter::BODY_PARAM_BOUNCE => self.set_bounce(value.to()),
            BodyParameter::BODY_PARAM_FRICTION => {
                self.set_friction(value.to());
            }
            BodyParameter::BODY_PARAM_MASS => {
                self.set_mass(value.to());
            }
            BodyParameter::BODY_PARAM_INERTIA => {
                self.set_inertia(value.to());
            }
            BodyParameter::BODY_PARAM_CENTER_OF_MASS => {
                self.set_center_of_mass(value.to());
            }
            BodyParameter::BODY_PARAM_GRAVITY_SCALE => {
                self.set_gravity_scale(value.to());
            }
            BodyParameter::BODY_PARAM_LINEAR_DAMP_MODE => {
                self.linear_damp_mode = value.to();
            }
            BodyParameter::BODY_PARAM_ANGULAR_DAMP_MODE => {
                self.angular_damp_mode = value.to();
            }
            BodyParameter::BODY_PARAM_LINEAR_DAMP => {
                self.set_linear_damp(value.to());
            }
            BodyParameter::BODY_PARAM_ANGULAR_DAMP => {
                self.set_angular_damp(value.to());
            }
            _ => {}
        };
    }

    pub fn set_state(&mut self, state: BodyState, value: &Variant) {
        match state {
            BodyState::BODY_STATE_TRANSFORM => self.set_transform(value.to()),
            BodyState::BODY_STATE_LINEAR_VELOCITY => self.set_linear_velocity(value.to()),
            BodyState::BODY_STATE_ANGULAR_VELOCITY => self.set_angular_velocity(value.to()),
            BodyState::BODY_STATE_SLEEPING => self.set_is_sleeping(value.to()),
            BodyState::BODY_STATE_CAN_SLEEP => self.set_can_sleep(value.to()),
            _ => {}
        };
    }

    pub fn set_transform(&mut self, value: Transform3D) {
        self.transform = value;

        if self.is_kinematic() {
            self.kinematic_isometry = transform_to_isometry(&value).0;
            return;
        }

        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                space.borrow_mut().set_transform(handle, value);
            }
        }
    }

    pub fn total_angular_damp(&self) -> f32 {
        let mut total_angular_damp = 0.0;
        let mut angular_damp_done = self.angular_damp_mode == DampMode::DAMP_MODE_REPLACE;
        for area in &self.areas {
            angular_damp_done = match area.borrow().angular_damp_mode() {
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_COMBINE => {
                    total_angular_damp += area.borrow().angular_damp();
                    false
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_COMBINE_REPLACE => {
                    total_angular_damp += area.borrow().angular_damp();
                    true
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_REPLACE => {
                    total_angular_damp = area.borrow().angular_damp();
                    true
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_REPLACE_COMBINE => {
                    total_angular_damp = area.borrow().angular_damp();
                    false
                }
                _ => false,
            };
            if angular_damp_done {
                break;
            }
        }

        if !angular_damp_done {
            if let Some(space) = self.space() {
                if let Some(default_area) = space.borrow().default_area() {
                    total_angular_damp += default_area.borrow().angular_damp();
                }
            }
        }
        match self.angular_damp_mode {
            DampMode::DAMP_MODE_COMBINE => total_angular_damp += self.angular_damp,
            DampMode::DAMP_MODE_REPLACE => total_angular_damp = self.angular_damp,
            _ => {}
        }

        total_angular_damp
    }

    pub fn total_gravity(&self) -> Vector3 {
        let mut gravity = Vector3::ZERO;
        let position = self.transform().origin;
        let mut gravity_done = false;
        for area in &self.areas {
            gravity_done = match area.borrow().gravity_mode() {
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_COMBINE => {
                    gravity += area.borrow().compute_gravity(position);
                    false
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_COMBINE_REPLACE => {
                    gravity += area.borrow().compute_gravity(position);
                    true
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_REPLACE => {
                    gravity = area.borrow().compute_gravity(position);
                    true
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_REPLACE_COMBINE => {
                    gravity = area.borrow().compute_gravity(position);
                    false
                }
                _ => false,
            };
            if gravity_done {
                break;
            }
        }
        if !gravity_done {
            if let Some(space) = self.space() {
                if let Some(default_area) = space.borrow().default_area() {
                    gravity += default_area.borrow().compute_gravity(position);
                }
            }
        }
        gravity *= self.gravity_scale;
        gravity
    }

    pub fn total_linear_damp(&self) -> f32 {
        let mut total_linear_damp = 0.0;
        let mut linear_damp_done = self.linear_damp_mode == DampMode::DAMP_MODE_REPLACE;
        for area in &self.areas {
            linear_damp_done = match area.borrow().linear_damp_mode() {
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_COMBINE => {
                    total_linear_damp += area.borrow().linear_damp();
                    false
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_COMBINE_REPLACE => {
                    total_linear_damp += area.borrow().linear_damp();
                    true
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_REPLACE => {
                    total_linear_damp = area.borrow().linear_damp();
                    true
                }
                AreaSpaceOverrideMode::AREA_SPACE_OVERRIDE_REPLACE_COMBINE => {
                    total_linear_damp = area.borrow().linear_damp();
                    false
                }
                _ => false,
            };
            if linear_damp_done {
                break;
            }
        }

        if !linear_damp_done {
            if let Some(space) = self.space() {
                if let Some(default_area) = space.borrow().default_area() {
                    total_linear_damp += default_area.borrow().linear_damp();
                }
            }
        }
        match self.linear_damp_mode {
            DampMode::DAMP_MODE_COMBINE => total_linear_damp += self.linear_damp,
            DampMode::DAMP_MODE_REPLACE => total_linear_damp = self.linear_damp,
            _ => {}
        }

        total_linear_damp
    }

    pub fn transform(&self) -> Transform3D {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                if let Some(body) = space.borrow().get_body(handle) {
                    return isometry_to_transform(body.position());
                }
            }
        }
        self.transform
    }

    pub fn update_damp(&self) {
        if let Some(space) = self.space() {
            if let Some(handle) = self.handle() {
                let total_linear_damp = self.total_linear_damp();
                let total_angular_damp = self.total_angular_damp();

                space
                    .borrow_mut()
                    .set_linear_damp(handle, total_linear_damp);
                space
                    .borrow_mut()
                    .set_angular_damp(handle, total_angular_damp);
            }
        }
    }
}

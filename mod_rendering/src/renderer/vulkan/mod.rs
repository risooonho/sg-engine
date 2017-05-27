
pub mod vertex;
use self::vertex::Vertex;

use vulkano_win;
use vulkano;
use cgmath;
use winit;

use vulkano_win::IntoVkWindowRef;
use vulkano_win::WindowRef;

use vulkano::buffer::BufferUsage;
use vulkano::buffer::CpuAccessibleBuffer;
use vulkano::command_buffer::DynamicState;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::command_buffer::CommandBufferBuilder;
use vulkano::device::Device;
use vulkano::framebuffer::Framebuffer;
use vulkano::framebuffer::Subpass;
use vulkano::instance::Instance;
use vulkano::instance::PhysicalDevice;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::pipeline::GraphicsPipelineParams;
use vulkano::pipeline::blend::Blend;
use vulkano::pipeline::depth_stencil::DepthStencil;
use vulkano::pipeline::input_assembly::InputAssembly;
use vulkano::pipeline::multisample::Multisample;
use vulkano::pipeline::vertex::SingleBufferDefinition;
use vulkano::pipeline::viewport::ViewportsState;
use vulkano::pipeline::viewport::Viewport;
use vulkano::pipeline::viewport::Scissor;
use vulkano::swapchain::SurfaceTransform;
use vulkano::swapchain::Surface;
use vulkano::swapchain::Swapchain;
use vulkano::pipeline::input_assembly::PrimitiveTopology;

use vulkano::image::attachment::AttachmentImage;
use vulkano::image::{
    ImmutableImage,
    SwapchainImage,
    ImageViewAccess,
    Image,
};

//use vulkano::device::QueuesIter;
use vulkano::device::Queue;
use vulkano::sync::GpuFuture;
use vulkano::descriptor::pipeline_layout::{
    PipelineLayout,
    PipelineLayoutDescUnion,
};
use vulkano::framebuffer::{
    RenderPassAbstract,
    FramebufferAbstract
};
use vulkano::pipeline::raster::{
    Rasterization,
    PolygonMode,
    CullMode,
    FrontFace,
    DepthBiasControl
};

// FIXME ju,k.u.m.[yu;j.7;i;.jk.7;.;;li
// k66kj,ku,,777777777777777777777777777777777777

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use std::collections::VecDeque;
use std::collections::hash_map::HashMap;

use ::renderer::utils::fps;

use game_state;
use game_state::{Identity, Identifyable, Renderer};
use game_state::input::InputSource;
use game_state::tree::{ BreadthFirstIterator };
use game_state::state::SceneGraph;

use image;

//TODO: compile these elsewhere, at build time?
// These shaders are a PITA, generated by build.rs, dependent on OUT_DIR... *barf
// More importantly, these are actually compiled SPIR-V, ignore the glsl file extension on them
mod vs { include!{concat!(env!("OUT_DIR"), "/assets/shaders/vs.glsl") }}
mod fs { include!{concat!(env!("OUT_DIR"), "/assets/shaders/fs.glsl") }}

pub struct BufferItem {
    pub vertices: Arc<CpuAccessibleBuffer<[Vertex]>>,
    pub indices: Arc<CpuAccessibleBuffer<[u16]>>,
    pub diffuse_map: Arc<CpuAccessibleBuffer<[[u8;4]]>>,
}

type ThisPipelineDescriptorSet = Arc<
        vulkano::descriptor::descriptor_set::SimpleDescriptorSet<
            (((),
              vulkano::descriptor::descriptor_set::SimpleDescriptorSetImg<
                  Arc<
                      vulkano::image::ImmutableImage<
                          vulkano::format::R8G8B8A8Unorm
                      >
                  >
              >
             ),
             vulkano::descriptor::descriptor_set::SimpleDescriptorSetBuf<
                 Arc<
                     vulkano::buffer::CpuAccessibleBuffer<
                         ::renderer::vulkan::vs::ty::Data
                     >
                 >
             >
            )
        >
    >;

type ThisPipelineType = Arc<
        GraphicsPipeline<
            SingleBufferDefinition<Vertex>,
            PipelineLayout<PipelineLayoutDescUnion<::renderer::vulkan::vs::Layout, ::renderer::vulkan::fs::Layout>>,
            Arc<RenderPassAbstract+Send+Sync>
        >
    >;

pub struct VulkanRenderer {
    id: Identity,
	_instance: Arc<Instance>,
	window: WindowRef,
    events_loop: Arc<Mutex<winit::EventsLoop>>,
	device: Arc<Device>,
	//queues: QueuesIter,
	queue: Arc<Queue>,
	swapchain: Arc<Swapchain>,
	_images: Vec<Arc<SwapchainImage>>,
	submissions: Vec<Box<GpuFuture>>,
	pipeline: ThisPipelineType,
	framebuffers: Vec<Arc<FramebufferAbstract + Send + Sync>>,//Vec<Arc<Framebuffer<render_pass::CustomRenderPass>>>,
    texture: Arc<vulkano::image::ImmutableImage<vulkano::format::R8G8B8A8Unorm>>,
	fps: fps::FPS,

    _renderpass: Arc<RenderPassAbstract + Send + Sync>,

    // descriptor set TODO: move this to BufferItem, so it can be associated with a mesh?
    pipeline_set: ThisPipelineDescriptorSet,//Arc<pipeline_layout::set0::Set>,

    _uniform_buffer: Arc<CpuAccessibleBuffer<::renderer::vulkan::vs::ty::Data>>,
    render_layer_queue: VecDeque<Arc<SceneGraph>>,
    buffer_cache: HashMap<usize, BufferItem>,

    rect: ScreenRect,
    current_mouse_pos: ScreenPoint,
    debug_world_rotation: f32,
}

#[allow(dead_code)]
pub enum DrawMode {
    Wireframe,
    Points,
    Colored
}

impl VulkanRenderer {

    fn create_swapchain(
        surface: &Arc<Surface>,
        device: &Arc<Device>,
        queue: &Arc<Queue>,
        physical: &PhysicalDevice
    ) -> (Arc<Swapchain>, Vec<Arc<SwapchainImage>>) {
        let caps = surface.get_capabilities(physical).expect("Failed to get surface capabilities");
        let dimensions = caps.current_extent.unwrap_or([1280, 800]);
        let present = caps.present_modes.iter().next().unwrap();
        let alpha = caps.supported_composite_alpha.iter().next().unwrap();
        let format = caps.supported_formats[0].0;
        Swapchain::new(
            device,
            surface,
            2,
            format,
            dimensions,
            1,
            &caps.supported_usage_flags,
            queue,
            SurfaceTransform::Identity,
            alpha,
            present,
            true,
            None
        ).expect("Failed to create swapchain.")
    }

    fn create_descriptor_set(
        texture: &Arc<ImmutableImage<vulkano::format::R8G8B8A8Unorm>>,
        device: &Arc<Device>,
        width: u32,
        height: u32,
        uniform_buffer: &Arc<CpuAccessibleBuffer<::renderer::vulkan::vs::ty::Data>>,
        queue: &Arc<Queue>,
        pipeline: &ThisPipelineType,
    ) -> ThisPipelineDescriptorSet {

        let sampler = vulkano::sampler::Sampler::new(&device, vulkano::sampler::Filter::Linear,
                                                     vulkano::sampler::Filter::Linear, vulkano::sampler::MipmapMode::Nearest,
                                                     vulkano::sampler::SamplerAddressMode::Repeat,
                                                     vulkano::sampler::SamplerAddressMode::Repeat,
                                                     vulkano::sampler::SamplerAddressMode::Repeat,
                                                     0.0, 1.0, 0.0, 0.0).unwrap();

        Arc::new(simple_descriptor_set!(pipeline.clone(), 0, {
            tex: (texture.clone(), sampler.clone()),
            uniforms: uniform_buffer.clone(),
        }))
    }

    // FIXME don't pass a tuple, rather a new struct type that composes these
	pub fn new(
        window_pair: (Arc<Mutex<winit::Window>>, Arc<Mutex<winit::EventsLoop>>),
        draw_mode: DrawMode
    ) -> Self {

		// Vulkan
		let instance = {
            use vulkano::instance::ApplicationInfo;
			let extensions = vulkano_win::required_extensions();
            let app_info = ApplicationInfo::from_cargo_toml();
			let i = Instance::new(Some(&app_info), &extensions, None).expect("Failed to create Vulkan instance. ");
            i
		};

		let physical = vulkano::instance::PhysicalDevice::enumerate(&instance)
			.next().expect("No device available.");

		let window: vulkano_win::WindowRef = window_pair.0.into_vk_win(&instance).expect("unable to create vk win");

        println!("getting queue");
		let queue = physical.queue_families().find(|q| {
			q.supports_graphics() && window.surface().is_supported(q).unwrap_or(false)
		}).expect("Couldn't find a graphical queue family.");

        println!("getting device");
		let (device, mut queues) = {
			let device_ext = vulkano::device::DeviceExtensions {
				khr_swapchain: true,
				.. vulkano::device::DeviceExtensions::none()
			};

			Device::new(&physical, physical.supported_features(), &device_ext,
				[(queue, 0.5)].iter().cloned()
			).expect("Failed to create device.")
		};

		let queue = queues.next().unwrap();
		let (swapchain, images) = Self::create_swapchain(&window.surface(), &device, &queue, &physical);

        // TODO: as part of asset_loader, we should be loading all the shaders we expect to use in a scene
		let vs = vs::Shader::load(&device).expect("failed to create vs shader module");
		let fs = fs::Shader::load(&device).expect("failed to create fs shader module");

        /// ----------------------------------
        /// Uniform buffer
        // TODO: extract to the notion of a camera
        let proj = cgmath::perspective(
            cgmath::Rad(::std::f32::consts::FRAC_PI_2),
            {
               let d = Image::dimensions(&images[0]);
               d.width() as f32 / d.height() as f32
            },
            0.01,
            100.0 // depth used for culling!
        );

        // Vulkan uses right-handed coordinates, y positive is down
        let view = cgmath::Matrix4::look_at(
            cgmath::Point3::new(0.0, 0.0, -4.0),   // eye
            cgmath::Point3::new(0.0, -5.5, 0.0),  // center
            cgmath::Vector3::new(0.0, -1.0, 0.0)  // up
        );

        let scale = cgmath::Matrix4::from_scale(1.0);

        let uniform_buffer = CpuAccessibleBuffer::<vs::ty::Data>::from_data(
            &device,
            &vulkano::buffer::BufferUsage::all(),
            Some(queue.family()),
            vs::ty::Data {
                world : <cgmath::Matrix4<f32> as cgmath::SquareMatrix>::identity().into(),
                view : (view * scale).into(),
                proj : proj.into(),
            }).expect("failed to create buffer");
        /// ----------------------------------

        let depth_buffer = Image::access(
            AttachmentImage::transient(
                &device,
                SwapchainImage::dimensions(&images[0]),
                vulkano::format::D16Unorm
            ).unwrap()
        );

        #[allow(dead_code)]
        let renderpass = single_pass_renderpass!(device.clone(),
                attachments: {
                    color: {
                        load: Clear,
                        store: Store,
                        format: Image::format(&images[0]),
                        samples: 1,
                    },
                    depth: {
                        load: Clear,
                        store: Store,
                        format: vulkano::image::ImageAccess::format(&depth_buffer),
                        samples: 1,
                    }
                },
                pass: {
                    color: [color],
                    depth_stencil: {depth}
                }
            ).unwrap();


        let renderpass_arc = Arc::new(renderpass); //as Arc<RenderPassAbstract + Send + Sync>;
        let depth_buffer = Arc::new(depth_buffer);

        let framebuffers = images.iter().map(|image| {
            //let attachments = renderpass_arc.desc().start_attachments()
            //    .color(image.clone()).depth(depth_buffer.clone());
            let dimensions = [Image::dimensions(image).width(), Image::dimensions(image).height(), 1];
            Framebuffer::new(
                renderpass_arc.clone(),
                dimensions,
                vec![ // because we are using RenderPassAbstract, we have to pass the Vec<Arc<ImageView + Send + Sync>>
                    image.clone() as Arc<ImageViewAccess + Send + Sync>,
                    depth_buffer.clone() as Arc<ImageViewAccess + Send + Sync>
                ]
            ).unwrap() as Arc<FramebufferAbstract + Send + Sync>
        }).collect::<Vec<_>>();

        // -----------------------------------------------
        // Rendermodes, fill, lines, points

        let polygonmode = match draw_mode {
            DrawMode::Colored => PolygonMode::Fill,
            DrawMode::Points  => PolygonMode::Point,
            DrawMode::Wireframe => PolygonMode::Line
        };

        let mut raster = Rasterization::default();
        raster.cull_mode = CullMode::Back;
        raster.polygon_mode = polygonmode;
        raster.depth_clamp = true;
        raster.front_face = FrontFace::Clockwise;
        raster.line_width = Some(2.0);
        raster.depth_bias = DepthBiasControl::Dynamic;
        // -------------------------------------------------

		let pipeline = Arc::new(GraphicsPipeline::new(&device, GraphicsPipelineParams {
			vertex_input: SingleBufferDefinition::new(),
			vertex_shader: vs.main_entry_point(),
			input_assembly: InputAssembly {
				topology: PrimitiveTopology::TriangleList,
				primitive_restart_enable: false,
			},
			tessellation: None,
			geometry_shader: None, //&geometry_shader,
			viewport: ViewportsState::Fixed {
				data: vec![(
					Viewport {
						origin: [0.0, 0.0],
						depth_range: 0.0 .. 1.0,
						dimensions: [Image::dimensions(&images[0]).width() as f32,
						Image::dimensions(&images[0]).height() as f32],
					},
					Scissor::irrelevant()
				)],
			},
			raster: raster,
			multisample: Multisample::disabled(),
			fragment_shader: fs.main_entry_point(),
			depth_stencil: DepthStencil::simple_depth_test(),
			blend: Blend::pass_through(),
			render_pass: Subpass::from(renderpass_arc.clone() as Arc<RenderPassAbstract + Send + Sync>, 0).unwrap(),
		}).unwrap());


        // TODO: texture sizes?

        let texture = ImmutableImage::new(&device, vulkano::image::Dimensions::Dim2d { width: 2048, height: 2048  },
                                          vulkano::format::R8G8B8A8Unorm, Some(queue.family())).unwrap();

        let pipeline_set = Self::create_descriptor_set(
            &texture, &device, 2048, 2048, &uniform_buffer, &queue, &pipeline
        );

		let submissions: Vec<Box<GpuFuture>> = Vec::new();
        // finish up by grabbing some initialization values for position and size
        let (x,y) = window.window().lock().unwrap().get_position().unwrap_or((0,0));
        let (w,h) = window.window().lock().unwrap().get_inner_size_pixels().unwrap_or((0,0));
        // TODO: get actual mouse position... or does it matter at this point when we get it in the
        // event loop instead

		VulkanRenderer {
            id: game_state::create_next_identity(),
            _instance: instance.clone(),
            window: window,
            events_loop: window_pair.1.clone(),
			device: device,
            //queues: queues,
			queue: queue,
			swapchain: swapchain,
            _images: images,
			submissions: submissions,
			pipeline: pipeline,
			framebuffers: framebuffers,
            texture: texture,
            _renderpass: renderpass_arc as Arc<RenderPassAbstract + Send + Sync>,
            pipeline_set: pipeline_set,

            fps: fps::FPS::new(),
            _uniform_buffer: uniform_buffer,
            render_layer_queue: VecDeque::new(),
            buffer_cache: HashMap::new(),


            current_mouse_pos: ScreenPoint::new(0, 0),
            rect: ScreenRect::new(x as i32, y as i32, w as i32, h as i32),
            debug_world_rotation: 0f32,
		}

	}

    #[inline]
    fn get_mouse_pos(&self) -> &ScreenPoint { &self.current_mouse_pos }

    #[inline]
    fn set_mouse_pos(&mut self, pos: ScreenPoint) { self.current_mouse_pos = pos; }

    #[allow(dead_code)]
    #[inline] fn get_rect(&self) -> &ScreenRect { &self.rect }

    #[inline]
    fn set_rect(&mut self, new_rect: ScreenRect) {
        // TODO: determine a delta here?
        // TODO: let the renderer know to change things up because we were resized?
        self.rect = new_rect;
    }

    #[inline]
    pub fn insert_buffer(&mut self, id: usize, vertices: &Vec<Vertex>, indices: &Vec<u16>, diffuse_map: &image::DynamicImage) {

        let pixel_buffer = {
            let image = diffuse_map.to_rgba();
            let image_data = image.into_raw().clone();

            let image_data_chunks = image_data.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]);

            // TODO: staging buffer instead
            vulkano::buffer::cpu_access::CpuAccessibleBuffer::<[[u8; 4]]>
            ::from_iter(&self.device, &vulkano::buffer::BufferUsage::all(),
                        Some(self.queue.family()), image_data_chunks)
                .expect("failed to create buffer")
        };

        self.buffer_cache.insert(id,
            BufferItem{
                vertices: CpuAccessibleBuffer::from_iter(
                    &self.device,
                    &BufferUsage::all(),
                    Some(self.queue.family()),
                    vertices.iter().cloned()
                ).expect("Unable to create buffer"),
                indices: CpuAccessibleBuffer::from_iter(
                    &self.device,
                    &BufferUsage::all(),
                    Some(self.queue.family()),
                    indices.iter().cloned()
                ).expect("Unable to create buffer"),
                diffuse_map: pixel_buffer
            }
        );
    }

    fn render(&mut self) {

        while self.submissions.len() >= 4 {
            self.submissions.remove(0);
        }

        let (image_num, future) = self.swapchain.acquire_next_image(Duration::new(1, 0)).unwrap();

        // todo: how are passes organized if textures must be uploaded first?
        // FIXME: use an initialization step rather than this quick hack
        // FIXME: that might look like a new method on Renderer - reload_buffers?

        let mut cmd_buffer_build = AutoCommandBufferBuilder::new(self.device.clone(), self.queue.family()).unwrap(); // catch oom error here
        {
            let maybe_buffer = self.buffer_cache.get(&0usize);
            match maybe_buffer {
                Some(item) => {
                    let &BufferItem { ref diffuse_map, .. } = item;
                    cmd_buffer_build = cmd_buffer_build.copy_buffer_to_image(diffuse_map.clone(), self.texture.clone())
                        .expect("unable to upload texture");
                },
                _ => {}
            }
        }

        cmd_buffer_build = cmd_buffer_build.begin_render_pass(
            self.framebuffers[image_num].clone(), false,
            //
            vec![
                vulkano::format::ClearValue::from([0.0,0.0,0.0,1.0]),
                vulkano::format::ClearValue::Depth(1.0)
            ]
        ).expect("unable to begin renderpass");

        loop {

            // TODO: implement a notion of a camera
            // TODO: that might be best done through the uniform_buffer, as it's what owns the
            // TODO: projection matrix at this point

            self.debug_world_rotation += 0.01;
            match self.render_layer_queue.pop_front() {
                Some(next_layer) => {

                    // TODO: load assets through mod_asset_loader, put into State
                    // TODO: refactor this to use asset lookups
                    // TODO: refactor this to use WorldEntity collection -> SceneGraph Rc types
                    // TODO: asset lookups should store DescriptorSets with associated textures

                    let iterator = BreadthFirstIterator::new(next_layer.root.clone());
                    for (_node_id, rc) in iterator {
                        let mut node = &mut rc.borrow_mut();

                        let model_mat = node.data.get_model_matrix().clone();
                        let rotation = cgmath::Matrix4::from_angle_y(cgmath::Rad(self.debug_world_rotation));
                        let rot_model = model_mat * rotation;
                        // TODO: updating the world matrices from the parent * child's local matrix
                        match node.parent() {
                            Some(parent) => {
                                let ref parent_model = parent.borrow().data;
                                let global_mat = parent_model.get_world_matrix() * rot_model;
                                node.data.set_world_matrix(global_mat);
                            },
                            None => {
                                node.data.set_world_matrix(rot_model);
                            }
                        }

                        let mesh = node.data.get_mesh();

                        if !self.buffer_cache.contains_key(&(node.data.identify() as usize)) {
                            let vertices: Vec<Vertex> = mesh.vertices.iter().map(|x| Vertex::from(*x)).collect();
                            self.insert_buffer(
                                node.data.identify() as usize,
                                &vertices,
                                &mesh.indices,
                                &node.data.get_diffuse_map()
                            );
                        }

                        let (v, i, _t) = {
                            let item = self.buffer_cache.get(&(node.data.identify() as usize)).unwrap();
                            (item.vertices.clone(), item.indices.clone(), item.diffuse_map.clone())
                        };

                        // Push constants are leveraged here to send per-model matrices into the shaders
                        let push = vs::ty::PushConstants {
                            model: node.data.get_world_matrix().clone().into()
                        };
                        // begin the command buffer
                        cmd_buffer_build = cmd_buffer_build.draw_indexed(
                                self.pipeline.clone(),
                                DynamicState::none(),
                                v.clone(),
                                i.clone(),
                                self.pipeline_set.clone(),
                                push
                        ).expect("Unable to add command");

                    }
                },
                None => break
            }
        }

        let cmd_buffer_build = cmd_buffer_build.end_render_pass();
        let cmd_buffer = cmd_buffer_build.expect("unable to end renderpass ").build().unwrap();

        let future = future.then_execute(self.queue.clone(), cmd_buffer);//.unwrap();
        let future = future.then_swapchain_present(self.queue.clone(), self.swapchain.clone(), image_num);
        let future = future.then_signal_fence();

        future.flush().unwrap();

        self.submissions.push(Box::new(future) as Box<_>);

        self.fps.update();
    }

    #[allow(dead_code)]
    fn fps(&self) -> f32 {
        self.fps.get()
    }

}


use game_state::input::events::{
    InputEvent,
    MouseButton,
};

use game_state::input::screen::{
    ScreenPoint,
    ScreenRect,
    DeltaVector,
};

impl Identifyable for VulkanRenderer {
    fn identify(&self) -> Identity {
        self.id
    }
}

impl Renderer for VulkanRenderer {
    fn load(&mut self) {
    }

    fn unload(&mut self) {
        self.buffer_cache.clear();
    }

    fn queue_render_layer(&mut self, layer: Arc<SceneGraph>) {
        self.render_layer_queue.push_back(layer);
    }

    fn present(&mut self) {
        self.render();
    }

    fn set_title(&mut self, title: &str) {
        let title = format!("{} (id: {})", title, self.id);
        self.window.window().lock().unwrap().set_title(&title );
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        println!("VulkanRenderer drop");
    }
}

impl InputSource for VulkanRenderer {
    fn get_input_events(&mut self) -> VecDeque<InputEvent> {
        use winit;

        let mut events = VecDeque::new();
        {
            let event_loop = &mut self.events_loop.lock().unwrap();
            event_loop.poll_events(|e| events.push_back(e.clone()));
        }

        let this_window_id = {
            let window = &mut self.window.window().lock().unwrap();
            window.id()
        };

        let mut converted_events = VecDeque::with_capacity(events.len());

        for e in events {
            match e {
                winit::Event::WindowEvent{ window_id, ref event } if window_id == this_window_id => {
                    let maybe_converted_event = match event {
                        // Keyboard Events
                        &winit::WindowEvent::KeyboardInput(state, scancode, _maybe_virtual_keycode, _modifier_state) => {
                            let e = match state {
                                winit::ElementState::Pressed => InputEvent::KeyDown(self.id, scancode),
                                winit::ElementState::Released => InputEvent::KeyDown(self.id, scancode)
                            };
                            Some(e)
                        }

                        // Mouse Events

                        &winit::WindowEvent::MouseMoved(x,y) => {
                            let old_pos: ScreenPoint = self.get_mouse_pos().clone();
                            let new_pos = ScreenPoint::new(x,y);
                            let moved =
                                InputEvent::MouseMove(self.id, new_pos.clone(), DeltaVector::from_points(&old_pos, &new_pos));
                            self.set_mouse_pos(new_pos);
                            Some(moved)
                        },
                        &winit::WindowEvent::MouseInput(state, button) => {
                            let b = match button {
                                winit::MouseButton::Left => MouseButton::Left,
                                winit::MouseButton::Right => MouseButton::Right,
                                winit::MouseButton::Middle => MouseButton::Middle,
                                winit::MouseButton::Other(n) => MouseButton::Other(n)
                            };
                            let e = match state {
                                winit::ElementState::Pressed => InputEvent::MouseDown(self.id, b, self.get_mouse_pos().clone()),
                                winit::ElementState::Released => InputEvent::MouseUp(self.id, b, self.get_mouse_pos().clone())
                            };
                            Some(e)
                        },

                        &winit::WindowEvent::MouseWheel(delta, _phase) => {
                            let e = match delta {
                                winit::MouseScrollDelta::LineDelta(x,y) | winit::MouseScrollDelta::PixelDelta(x,y)  =>
                                    InputEvent::MouseWheel(self.id, self.get_mouse_pos().clone(), DeltaVector::new(x as i32, y as i32))
                            };
                            Some(e)
                        },

                        // Window Manager events

                        &winit::WindowEvent::MouseEntered => Some(InputEvent::MouseEntered(self.id)),
                        &winit::WindowEvent::MouseLeft => Some(InputEvent::MouseLeft(self.id)),
                        &winit::WindowEvent::Closed => Some(InputEvent::Closed(self.id)),
                        &winit::WindowEvent::Focused(f) => Some(if f { InputEvent::GainedFocus(self.id) } else { InputEvent::LostFocus(self.id) }),
                        &winit::WindowEvent::Moved(x,y) => {
                            let new_rect = ScreenRect::new(x as i32, y as i32, self.rect.w, self.rect.h);
                            let e = InputEvent::Moved(self.id, ScreenPoint::new(x as i32, y as i32));
                            self.set_rect(new_rect);
                            Some(e)
                        }
                        &winit::WindowEvent::Resized(w, h) => {
                            let new_rect = ScreenRect::new(self.rect.x, self.rect.y, w as i32, h as i32);
                            let e = InputEvent::Resized(self.id, new_rect.clone());
                            self.set_rect(new_rect);
                            Some(e)
                        },
                        _ => None

                    };
                    if maybe_converted_event.is_some() {
                        converted_events.push_back(maybe_converted_event.unwrap());
                    }

                }
                _ => {}
            };
        }
        converted_events
    }
    // FIXME Ruby
}

#[cfg(test)]
mod tests {

    #[test]
    fn rando_test_flatten_vec_of_options(){
        let vals = vec![None, None, Some(1), None, Some(2), Some(3), None, None, None, Some(4)];
        let flat = vals.iter().enumerate().filter(|&(_, x)| x.is_some()).map(|(_, x)| x.unwrap()).collect::<Vec<u32>>();
        assert_eq!(flat, vec![1,2,3,4]);
    }
}


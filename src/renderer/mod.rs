pub mod utils;
pub mod vertex;

use self::vertex::Vertex;

extern crate winit;
extern crate vulkano;
extern crate vulkano_win;
extern crate cgmath;
extern crate time;

use vulkano_win::VkSurfaceBuild;
use vulkano::buffer::BufferUsage;
use vulkano::buffer::CpuAccessibleBuffer;
use vulkano::buffer::ImmutableBuffer;
use vulkano::buffer::DeviceLocalBuffer;
use vulkano::command_buffer;
use vulkano::command_buffer::DynamicState;
use vulkano::command_buffer::PrimaryCommandBufferBuilder;
use vulkano::command_buffer::Submission;
use vulkano::device::Device;
use vulkano::framebuffer::Framebuffer;
use vulkano::framebuffer::Subpass;
use vulkano::instance::Instance;
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
use vulkano::swapchain::Swapchain;
use vulkano::pipeline::input_assembly::PrimitiveTopology;
use vulkano::image::SwapchainImage;
use vulkano::device::QueuesIter;
use vulkano::device::Queue;

use std::sync::Arc;
use std::time::Duration;

use std::collections::VecDeque;

use self::utils::fps;

use game_state::{Renderer, Renderable};
use game_state::state::SceneGraph;

//TODO: compile these elsewhere, at build time?
// These shaders are a PITA, generated by build.rs, dependent on OUT_DIR... *barf
// More importantly, these are actually compiled SPIR-V, ignore the glsl file extension on them
mod vs { include!{concat!(env!("OUT_DIR"), "/shaders/assets/shaders/triangle_vs.glsl") }}
mod fs { include!{concat!(env!("OUT_DIR"), "/shaders/assets/shaders/triangle_fs.glsl") }}

// TODO: do we want clearcolor settable?
const CLEAR_COLOR: [f32;4] = [0.0, 0.0, 0.0, 1.0];

// TODO: explore more complex options for renderpasses
pub mod render_pass {
	use vulkano::format::Format;
	single_pass_renderpass!{
		attachments: {
			color: {
				load:Clear,
				store:Store,
				format:Format,
			},
			depth: {
			    load: Clear,
			    store: DontCare,
			    format: ::vulkano::format::D16Unorm,
			}
		},
		pass: {
			color: [color],
			depth_stencil: {depth}
		}
	}
}

pub mod pipeline_layout {
    pipeline_layout! {
        set0: {
            uniforms: UniformBuffer<::renderer::vs::ty::Data>
        }
    }
}

pub struct VulkanRenderer {
	instance: Arc<Instance>,
	window: vulkano_win::Window,
	device: Arc<Device>,
	queues: QueuesIter,
	queue: Arc<Queue>,
	swapchain: Arc<Swapchain>,
	images: Vec<Arc<SwapchainImage>>,
	submissions: Vec<Arc<Submission>>,
	pipeline: Arc<GraphicsPipeline<SingleBufferDefinition<Vertex>, pipeline_layout::CustomPipeline, render_pass::CustomRenderPass>>,
	framebuffers: Vec<Arc<Framebuffer<render_pass::CustomRenderPass>>>,
	render_pass: Arc<render_pass::CustomRenderPass>,
	fps: fps::FPS,
    render_layer_queue: VecDeque<Arc<SceneGraph>>,
    pipeline_set: Arc<pipeline_layout::set0::Set>,
    uniform_buffer: Arc<CpuAccessibleBuffer<::renderer::vs::ty::Data>>,
}

impl VulkanRenderer {
	pub fn new(title: &str, h: u32, w: u32) -> Self {
		// Vulkan
		let instance = {
			let extensions = vulkano_win::required_extensions();
			Instance::new(None, &extensions, None).expect("Failed to create Vulkan instance.")
		};

		let physical = vulkano::instance::PhysicalDevice::enumerate(&instance)
			.next().expect("No device available.");

		let window = winit::WindowBuilder::new()
            .with_title(title)
            .with_dimensions(h,w)
            .build_vk_surface(&instance).unwrap();

		let queue = physical.queue_families().find(|q| {
			q.supports_graphics() && window.surface().is_supported(q).unwrap_or(false)
		}).expect("Couldn't find a graphical queue family.");

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

		let (swapchain, images) = {
			let caps = window.surface().get_capabilities(&physical).expect("Failed to get surface capabilities");
			let dimensions = caps.current_extent.unwrap_or([1280, 800]);
			let present = caps.present_modes.iter().next().unwrap();
			let alpha = caps.supported_composite_alpha.iter().next().unwrap();
			let format = caps.supported_formats[0].0;
			Swapchain::new(
				&device,
				&window.surface(),
				2,
				format,
				dimensions,
				1,
				&caps.supported_usage_flags,
				&queue,
				SurfaceTransform::Identity,
				alpha,
				present,
				true,
				None
			).expect("Failed to create swapchain.")
		};

		let vs = vs::Shader::load(&device).expect("failed to create vs shader module");
		let fs = fs::Shader::load(&device).expect("failed to create fs shader module");

		let render_pass = render_pass::CustomRenderPass::new(&device, &render_pass::Formats {
			color: (images[0].format(), 1),
            depth: (vulkano::format::D16Unorm, 1)
		}).unwrap();

        let proj = cgmath::perspective(
            cgmath::Rad(::std::f32::consts::FRAC_PI_2),
            { let d = images[0].dimensions(); d[0] as f32 / d[1] as f32 },
            0.01,
            5.0
        );

        // Vulkan uses right-handed coordinates, y positive is down
        let view = cgmath::Matrix4::look_at(
            cgmath::Point3::new(0.5, -1.0, 1.0),   // eye
            cgmath::Point3::new(0.0, 0.5, -1.0),  // center
            cgmath::Vector3::new(0.0, 1.0, 0.0)  // up
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

        let descriptor_pool = vulkano::descriptor::descriptor_set::DescriptorPool::new(&device);
        let pipeline_layout = pipeline_layout::CustomPipeline::new(&device).unwrap();
        let pipeline_set = pipeline_layout::set0::Set::new(&descriptor_pool, &pipeline_layout, &pipeline_layout::set0::Descriptors {
            uniforms: &uniform_buffer
        });

		let pipeline = GraphicsPipeline::new(&device, GraphicsPipelineParams {
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
						dimensions: [images[0].dimensions()[0] as f32,
						images[0].dimensions()[1] as f32],
					},
					Scissor::irrelevant()
				)],
			},
			raster: Default::default(),
			multisample: Multisample::disabled(),
			fragment_shader: fs.main_entry_point(),
			depth_stencil: DepthStencil::disabled(),
			blend: Blend::pass_through(),
			layout: &pipeline_layout,
			render_pass: Subpass::from(&render_pass, 0).unwrap(),
		}).unwrap();

        let depth_buffer = vulkano::image::attachment::AttachmentImage::transient(
            &device,
            images[0].dimensions(),
            vulkano::format::D16Unorm
        ).unwrap();

		let framebuffers = images.iter().map(|image| {
			let dimensions = [image.dimensions()[0], image.dimensions()[1], 1];
			Framebuffer::new(&render_pass, dimensions, render_pass::AList {
				color: &image,
                depth: &depth_buffer
			}).unwrap()
		}).collect::<Vec<_>>();

		let submissions:Vec<Arc<Submission>> = Vec::new();

		VulkanRenderer {
			// render
			device: device,
			queue: queue,
			swapchain: swapchain,
			submissions: submissions,
			pipeline: pipeline,
			framebuffers: framebuffers,
			render_pass: render_pass,

			//individually exposed
			images: images,
			queues: queues,
			instance: instance.clone(),
			fps: fps::FPS::new(),
			window: window,
            render_layer_queue: VecDeque::new(),
            pipeline_set: pipeline_set,
            uniform_buffer: uniform_buffer,
		}

	}


	pub fn instance(&self) -> Arc<Instance> {
		self.instance.clone()
	}

	pub fn queues(&mut self) -> &mut QueuesIter {
		&mut self.queues
	}

	pub fn images(&mut self) -> &Vec<Arc<SwapchainImage>> {
		&mut self.images
	}

	pub fn window(&self) -> &vulkano_win::Window {
		&self.window
	}

	pub fn native_window(&self) -> &winit::Window {
		&self.window.window()
	}

    pub fn create_cpu_buffer<T:'static+Sized+Clone>(&self, data: &Vec<T>) -> Arc<CpuAccessibleBuffer<[T]>> {
        CpuAccessibleBuffer::from_iter(
            &self.device,
            &BufferUsage::all(),
            Some(self.queue.family()),
            data.iter().cloned()
        ).expect("Unable to create buffer")
    }

    fn render(&mut self) {

        // TODO: create buffers for and setup draw calls

        self.submissions.retain(|s| s.destroying_would_block());
        let image_num = self.swapchain.acquire_next_image(Duration::new(1, 0)).unwrap();

        // begin the command buffer
        let mut cmd_buffer_build = PrimaryCommandBufferBuilder::new(&self.device, self.queue.family())
            .draw_inline(
                &self.render_pass,
                &self.framebuffers[image_num],
                render_pass::ClearValues {
                    color: CLEAR_COLOR,
                    depth: 1.0,
                });

        let mut rad = 0.00001;

        // TODO: do away with this renderable queue
        loop {
            match self.render_layer_queue.pop_front() {
                Some(next_layer) => {


                    // println!("got renderable");
                    let mesh = next_renderable.get_mesh();

                    let world_mat = next_renderable.get_world_matrix();
                    let view_mat = next_renderable.get_view_matrix();

                    let vertices: Vec<Vertex> = mesh.vertices.iter().map(|x| {
                        let vertex: Vertex = x.into(); vertex
                    }).collect();

                    let indices = &mesh.indices;

                    let vert_buffer = self.create_cpu_buffer(&vertices);
                    let index_buffer = self.create_cpu_buffer(indices);

                    let mut buffer_content = self.uniform_buffer.write(
                        Duration::new(1, 0)
                    ).unwrap();

                    rad += 0.1;
                    let current_view: cgmath::Matrix4<f32> = buffer_content.view.into();
                    let rotation = cgmath::Matrix4::from_angle_y(cgmath::Rad(rad));
                    buffer_content.view = (current_view * rotation).into();
                    buffer_content.world = world_mat.clone().into();

                    //println!("building indexed command buffer");
                    cmd_buffer_build = cmd_buffer_build.draw_indexed(
                        &self.pipeline,
                        &vert_buffer,
                        &index_buffer,
                        &DynamicState::none(), &self.pipeline_set, &()
                    );
                },
                None => { break; }
            }
        }
        //println!("draw_end() for command buffer");
        let cmd_buffer_build = cmd_buffer_build.draw_end();

        //println!("finalizing command buffer");
        let cmd_buffer = cmd_buffer_build.build();

        //println!("submitting command buffer");
        self.submissions.push(command_buffer::submit(&cmd_buffer, &self.queue).unwrap());

        //println!("presenting");
        self.swapchain.present(&self.queue, image_num).unwrap();

        self.fps.update();
    }

    #[allow(dead_code)]
    fn fps(&self) -> f32 {
        self.fps.get()
    }

}

impl Renderer for VulkanRenderer {
    fn queue_render_layer(&mut self, render_layer: Arc<SceneGraph>) {
        self.render_layer_queue.push_back(render_layer);
    }

    fn present(&mut self) {
        self.render();
    }
}


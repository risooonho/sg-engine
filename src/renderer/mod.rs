pub mod utils;

extern crate winit;
extern crate vulkano;
extern crate vulkano_win;

use vulkano_win::VkSurfaceBuild;
use vulkano::buffer::BufferUsage;
use vulkano::buffer::CpuAccessibleBuffer;
use vulkano::command_buffer;
use vulkano::command_buffer::DynamicState;
use vulkano::command_buffer::PrimaryCommandBufferBuilder;
use vulkano::command_buffer::Submission;
use vulkano::descriptor::pipeline_layout::EmptyPipeline;
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

use self::utils::fps;

use game_state::{Renderer, Renderable};
use game_state::model::{ GVertex };

//TODO: compile these elsewhere, at build time?
// These shaders are a PITA, generated by build.rs, dependent on OUT_DIR... *barf
// More importantly, these are actually compiled SPIR-V, ignore the glsl file extension on them
mod vs { include!{concat!(env!("OUT_DIR"), "/shaders/assets/shaders/triangle_vs.glsl") }}
mod fs { include!{concat!(env!("OUT_DIR"), "/shaders/assets/shaders/triangle_fs.glsl") }}

const CLEAR_COLOR: [f32;4] = [0.0, 0.0, 0.0, 1.0];
	
pub mod render_pass {
	use vulkano::format::Format;
	single_pass_renderpass!{
		attachments: {
			color: {
				load:Clear,
				store:Store,
				format:Format,
			}
		},
		pass: {
			color: [color],
			depth_stencil: {}
		}
	}
}

#[derive(Debug, Clone)]
pub struct Vertex {
	position: [f32;3],
	color: [f32;4]
}
impl_vertex!(Vertex, position, color); // passing arguments to shaders here
impl Vertex {
	pub fn new(position: [f32;3], color: [f32;4] ) -> Self {
		Vertex { position: position, color: color}
	}
    pub fn from(vert: GVertex) -> Self { // implies copying :(
        Vertex { position: vert.position, color: vert.color }
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
	pipeline: Arc<GraphicsPipeline<SingleBufferDefinition<Vertex>, EmptyPipeline, render_pass::CustomRenderPass>>,
	framebuffers: Vec<Arc<Framebuffer<render_pass::CustomRenderPass>>>,
	render_pass: Arc<render_pass::CustomRenderPass>,
	fps: fps::FPS,
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
			color: (images[0].format(), 1)
		}).unwrap();

		let pipeline = GraphicsPipeline::new(&device, GraphicsPipelineParams {
			vertex_input: SingleBufferDefinition::new(),
			vertex_shader: vs.main_entry_point(),
			input_assembly: InputAssembly {
				topology: PrimitiveTopology::TriangleStrip,
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
			layout: &EmptyPipeline::new(&device).unwrap(),
			render_pass: Subpass::from(&render_pass, 0).unwrap(),
		}).unwrap();

		let framebuffers = images.iter().map(|image| {
			let dimensions = [image.dimensions()[0], image.dimensions()[1], 1];
			Framebuffer::new(&render_pass, dimensions, render_pass::AList {
				color: image
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
    // TODO: Stop passing a vertex_buffer to render.
    //  Maybe setup a buffer to add things, then render state?
    fn render(&mut self, vertices: Vec<Vertex>) {
        let vertex_buffer: Arc<CpuAccessibleBuffer<[Vertex]>> =
            CpuAccessibleBuffer::from_iter(
                &self.device,
                &BufferUsage::all(),
                Some(self.queue.family()),
                vertices.as_slice().iter().cloned() // we end up copying them?
            ).expect("Failed to create vertex buffer");

        self.fps.update();
        self.submissions.retain(|s| s.destroying_would_block() );
        let image_num = self.swapchain.acquire_next_image(Duration::new(1,0)).unwrap();
        let command_buffer = PrimaryCommandBufferBuilder::new(&self.device, self.queue.family())
            .draw_inline(&self.render_pass, &self.framebuffers[image_num], render_pass::ClearValues {
                color: CLEAR_COLOR
            })
            .draw(&self.pipeline, &vertex_buffer, &DynamicState::none(), (), &())
            .draw_end()
            .build();

        self.submissions.push(command_buffer::submit(&command_buffer, &self.queue).unwrap());
        self.swapchain.present(&self.queue, image_num).unwrap();
    }

    fn fps(&self) -> f32 {
        self.fps.get()
    }

}

impl Renderer for VulkanRenderer {
    fn draw(&mut self, renderables: &Vec<Box<Renderable>>) {
        let mut vertices = Vec::with_capacity(renderables.len());
        for ref renderable in renderables {
            let geom = renderable.get_geometry().clone();
            for v in geom {
                vertices.push(Vertex::from(v));
            }
        }
        self.render(vertices);
    }
}


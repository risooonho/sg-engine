
pub mod vertex;
use self::vertex::Vertex;

pub mod vulkano_win_patch;

use vulkano;
use cgmath;
use game_state::winit;


use vulkano::buffer::BufferUsage;
use vulkano::buffer::CpuAccessibleBuffer;

use vulkano::image::ImageLayout;
use vulkano::image::MipmapsCount;

use vulkano::command_buffer::DynamicState;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::command_buffer::CommandBuffer;

use vulkano::descriptor::descriptor_set::{
    DescriptorSet,
    PersistentDescriptorSet,
    PersistentDescriptorSetBuf,
    PersistentDescriptorSetImg
};
use vulkano::descriptor::pipeline_layout::PipelineLayoutAbstract;
use vulkano::device::Device;
use vulkano::framebuffer::Framebuffer;
use vulkano::framebuffer::Subpass;
use vulkano::instance::Instance;
use vulkano::instance::PhysicalDevice;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::pipeline::depth_stencil::DepthStencil;
use vulkano::pipeline::vertex::SingleBufferDefinition;
use vulkano::pipeline::viewport::Viewport;
use vulkano::pipeline::viewport::Scissor;
use vulkano::swapchain;
use vulkano::swapchain::SurfaceTransform;
use vulkano::swapchain::Surface;
use vulkano::swapchain::Swapchain;
use vulkano::pipeline::input_assembly::PrimitiveTopology;

use vulkano::sync::now;

use vulkano::instance::debug::DebugCallback;

use vulkano::image::attachment::AttachmentImage;
use vulkano::image::{
    ImmutableImage,
    SwapchainImage,
    ImageViewAccess,
    ImageAccess,
    ImageUsage,
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
use std::mem;

use std::collections::VecDeque;

use game_state;
use game_state::utils::fps;
use game_state::{Identity, Identifyable, Renderer};
use game_state::input::InputSource;
use game_state::tree::{ BreadthFirstIterator };
use game_state::state::SceneGraph;
use game_state::state::DrawMode;
use game_state::model::Model;
use game_state::thing::Thing;
use game_state::thing::CameraFacet;

use image;

//TODO: compile these elsewhere, at build time?
// These shaders are a PITA, generated by build.rs, dependent on OUT_DIR... *barf
// More importantly, these are actually compiled SPIR-V, ignore the glsl file extension on them
mod vs { include!{concat!(env!("OUT_DIR"), "/assets/shaders/vs.glsl") }}
mod fs { include!{concat!(env!("OUT_DIR"), "/assets/shaders/fs.glsl") }}

// ModelData is intented to encapsulate all Model+Material data that's specific to this
// Vulkano renderer - geometry, indices, materials
pub struct ModelData {
    pub model: Arc<Model>,
    pub vertices: Arc<CpuAccessibleBuffer<[Vertex]>>,
    pub indices: Arc<CpuAccessibleBuffer<[u16]>>,
    pub diffuse_map: Arc<CpuAccessibleBuffer<[[u8;4]]>>,
    pub material_data: MaterialRenderData<vulkano::format::R8G8B8A8Unorm>
}

// MaterialData holds the Vulkano handles to GPU images - `init` and `read` here alias the same
// image, however init is used to write the data, while read is used to read
// the descriptor_set is used to bind on a per-model basis during traversal of the scene graph
pub struct MaterialRenderData<F> {
    pub read: Arc<ImmutableImage<F>>,
    pub init: Arc<ImageAccess>,
    pub descriptor_set: Arc<DescriptorSet + Send + Sync>
}

impl <F> MaterialRenderData<F> {
    pub fn new(
        read: Arc<ImmutableImage<F>>,
        init: Arc<ImageAccess>,
        descriptor_set: Arc<DescriptorSet+Send+Sync>
    ) -> Self {
        MaterialRenderData{ read, init, descriptor_set }
    }
}


type ThisPipelineType =
    GraphicsPipeline<
        SingleBufferDefinition<::renderer::vulkano::vertex::Vertex>,
        Box<PipelineLayoutAbstract + Send + Sync>,
        Arc<vulkano::framebuffer::RenderPassAbstract + Send + Sync>
    >;


type AMWin = Arc<winit::Window>;
pub struct VulkanoRenderer {
    id: Identity,
    instance: Arc<Instance>,

    #[allow(dead_code)]
    window: AMWin,

    #[allow(dead_code)]
    surface: Arc<Surface<AMWin>>,
    depth_buffer: Arc<ImageViewAccess + Send + Sync>,
    events_loop: Arc<Mutex<winit::EventsLoop>>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    swapchain: Arc<Swapchain<AMWin>>,
    images: Vec<Arc<SwapchainImage<AMWin>>>,
    pipeline: Arc<ThisPipelineType>,
    framebuffers: Vec<Arc<FramebufferAbstract + Send + Sync>>,
    fps: fps::FPS,

    renderpass: Arc<RenderPassAbstract + Send + Sync>,

    // TODO: camera
    uniform_buffer: Arc<CpuAccessibleBuffer<::renderer::vulkano::vs::ty::Data>>,

    render_layer_queue: VecDeque<Arc<SceneGraph>>,
    model_data: Vec<ModelData>,

    rect: ScreenRect,
    current_mouse_pos: ScreenPoint,
    debug_world_rotation: f32,
    debug_zoom: f32,

    // Enable vulkan debug layers? - need to install the vulkan sdk to get them
    #[allow(dead_code)]
    debug_callback: Option<vulkano::instance::debug::DebugCallback>,

    previous_frame_end: Box<GpuFuture>,
    recreate_swapchain: bool,
    dynamic_state: DynamicState,
}

impl VulkanoRenderer {

    fn create_swapchain(
        surface: Arc<Surface<AMWin>>,
        device: Arc<Device>,
        queue: Arc<Queue>,
        physical: PhysicalDevice
    ) -> Result<(Arc<Swapchain<AMWin>>, Vec<Arc<SwapchainImage<AMWin>>>), String> {

        let caps = match surface.capabilities(physical.clone()) {
            Ok(caps) => caps,
            Err(err) => {
                return Err(format!("Unable to get capabilities from surface: {:?}", err).to_string())
            }
        };

        use vulkano::swapchain::PresentMode;

        let dimensions = caps.current_extent.unwrap_or([1280, 800]);
        let present = caps.present_modes.iter().next().unwrap();
        let alpha = caps.supported_composite_alpha.iter().next().unwrap();
        let format = caps.supported_formats[0].0;

        // note that some present modes block on vsync
        // TODO: this should be a user-configurable option
        // THOUGHTS: perhaps this could be better supported by putting the renderer on another thread
        // and then syncing with state once per update, but allowing rendering to happen
        // without blocking
        let present_mode = if caps.present_modes.immediate {
            Some(PresentMode::Immediate)
        } else if caps.present_modes.mailbox {
            Some(PresentMode::Mailbox)
        } else if caps.present_modes.relaxed {
            Some(PresentMode::Relaxed)
        } else if caps.present_modes.mailbox {
            Some(PresentMode::Fifo)
        } else {
            None
        }.expect("No supported present mode found.");

        Ok(Swapchain::new(
            device,
            surface,
            caps.min_image_count,
            format,
            dimensions,
            1,
            caps.supported_usage_flags,
            &queue,
            SurfaceTransform::Identity,
            alpha,
            present_mode,
            true,
            None
        ).expect("Failed to create swapchain."))
    }

    fn create_descriptor_set(
        device: Arc<Device>,
        uniform_buffer: Arc<CpuAccessibleBuffer<::renderer::vulkano::vs::ty::Data>>,
        queue: Arc<Queue>,
        pipeline: Arc<ThisPipelineType>,
        id: usize,
        texture: Arc<ImmutableImage<vulkano::format::R8G8B8A8Unorm>>,
        width: u32,
        height: u32,
    ) -> Arc<DescriptorSet + Send + Sync> {

        let sampler = vulkano::sampler::Sampler::new(
            device.clone(),
            vulkano::sampler::Filter::Linear,
            vulkano::sampler::Filter::Linear,
            vulkano::sampler::MipmapMode::Nearest,
            vulkano::sampler::SamplerAddressMode::Repeat,
            vulkano::sampler::SamplerAddressMode::Repeat,
            vulkano::sampler::SamplerAddressMode::Repeat,
            0.0, 1.0, 0.0, 0.0
        ).unwrap();

        let ds = PersistentDescriptorSet::start(pipeline, 0) // intended to be bound at 0
            .add_sampled_image(texture, sampler)
            .expect("error loading texture")
            .add_buffer(uniform_buffer)
            .expect("error adding uniform buffer")
            .build()
            .unwrap();

        Arc::new(ds) as Arc<DescriptorSet + Send + Sync>

    }

    fn create_framebuffers(
        renderpass: Arc<RenderPassAbstract + Send + Sync>,
        images: Vec<Arc<SwapchainImage<AMWin>>>,
        depth_buffer: Arc<ImageViewAccess + Send + Sync>
    ) -> Vec<Arc<FramebufferAbstract + Send + Sync>> {
        images.iter().map(|image| {
            let dimensions = [
                ImageAccess::dimensions(image).width(),
                ImageAccess::dimensions(image).height(),
                1
            ];
            let fb =
                Framebuffer::with_dimensions(renderpass.clone(), dimensions)
                .add( image.clone() as Arc<ImageViewAccess + Send + Sync>)
                .unwrap()
                .add( depth_buffer.clone() as Arc<ImageViewAccess + Send + Sync> )
                .unwrap()
                .build()
                .unwrap();
            Arc::new(fb) as Arc<FramebufferAbstract + Send + Sync>
        }).collect::<Vec<_>>()
    }


    pub fn new<'a>(
        window: AMWin,
        events_loop: Arc<Mutex<winit::EventsLoop>>,
        draw_mode: DrawMode
    ) -> Result<Self, String>{

        let instance = {
            let extensions = vulkano_win_patch::required_extensions();
            let app_info = app_info_from_cargo_toml!();
            let i = Instance::new(Some(&app_info), &extensions, None).expect("Failed to create Vulkan instance. ");
            i
        };

        let debug_callback = DebugCallback::errors_and_warnings(
            &instance, |msg| {
                println!("Debug callback: {:?}", msg.description);
            }
        ).ok();

        let physical = vulkano::instance::PhysicalDevice::enumerate(&instance)
            .next().expect("No device available.");

        let surface: Arc<Surface<AMWin>> = unsafe {
           match vulkano_win_patch::winit_to_surface(instance.clone(), window.clone()) {
               Ok(s) => s,
               Err(e) => return Err("unable to create surface..".to_string())
           }
        };

        let queue = physical.queue_families().find(|q| {
            q.supports_graphics() && surface.is_supported(q.clone()).unwrap_or(false)
        }).expect("Couldn't find a graphical queue family.");

        let (device, mut queues) = {
            let device_ext = vulkano::device::DeviceExtensions {
                khr_swapchain: true,
                .. vulkano::device::DeviceExtensions::none()
            };

            Device::new(physical, physical.supported_features(), &device_ext,
                [(queue, 0.5)].iter().cloned()
            ).expect("Failed to create device.")
        };

        let queue = queues.next().unwrap();

        let (swapchain, images) = Self::create_swapchain(surface.clone(), device.clone(), queue.clone(), physical)?;

        // TODO: as part of asset_loader, we should be loading all the shaders we expect to use in a scene
        let vs = vs::Shader::load(device.clone()).expect("failed to create vs shader module");
        let fs = fs::Shader::load(device.clone()).expect("failed to create fs shader module");

        // ----------------------------------
        // Uniform buffer
        // TODO: extract to the notion of a camera
        
        let proj = cgmath::perspective(
            cgmath::Rad(::std::f32::consts::FRAC_PI_2),
            {
               let d = ImageAccess::dimensions(&images[0]);
               d.width() as f32 / d.height() as f32
            },
            0.01,
            100.0 // depth used for culling!
        );

        let uniform_buffer = CpuAccessibleBuffer::<vs::ty::Data>::from_data(
            device.clone(),
            vulkano::buffer::BufferUsage::all(),
            vs::ty::Data {
                proj : proj.into(),
            }
        ).expect("failed to create buffer");

        // ----------------------------------

        let img_usage = ImageUsage {
            transient_attachment: true,
            input_attachment: true,
            ..ImageUsage::none()
        };
        let depth_buffer = AttachmentImage::with_usage(
            device.clone(),
            SwapchainImage::dimensions(&images[0]),
            vulkano::format::D16Unorm,
            img_usage
        ).unwrap();

        #[allow(dead_code)]
        let renderpass = single_pass_renderpass!(device.clone(),
                attachments: {
                    color: {
                        load: Clear,
                        store: Store,
                        format: ImageAccess::format(&images[0]),
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


        let renderpass = Arc::new(renderpass); //as Arc<RenderPassAbstract + Send + Sync>;
        let depth_buffer = Arc::new(depth_buffer); // as Arc<ImageViewAccess + Send + Sync>
        let dimensions = ImageAccess::dimensions(&images[0]);
        let framebuffers = Self::create_framebuffers(renderpass.clone(), images.clone(), depth_buffer.clone());

        // -----------------------------------------------
        // Rendermodes, fill, lines, points
        let mut raster = Rasterization::default();
        raster.cull_mode = CullMode::Back;
        raster.polygon_mode = match draw_mode {
            DrawMode::Colored => PolygonMode::Fill,
            DrawMode::Points  => PolygonMode::Point,
            DrawMode::Wireframe => PolygonMode::Line
        };
        raster.depth_clamp = true;
        raster.front_face = FrontFace::Clockwise;
        raster.line_width = Some(2.0);
        raster.depth_bias = DepthBiasControl::Dynamic;
        // -------------------------------------------------

        let p = GraphicsPipeline::start()
            .vertex_input_single_buffer()
            .cull_mode_back()
            .polygon_mode_fill()
            .depth_clamp(true)
            .front_face_clockwise()
            .line_width(2.0)
            .vertex_shader(vs.main_entry_point(), ())
            .triangle_list()
            .viewports_dynamic_scissors_irrelevant(1)
            .fragment_shader(fs.main_entry_point(), ())
            .depth_stencil_simple_depth()
            .blend_alpha_blending()
            .render_pass(
                Subpass::from(
                    renderpass.clone() as Arc<RenderPassAbstract + Send + Sync>,
                    0
                ).unwrap()
            )
            .build(device.clone())
            .unwrap();

        let pipeline = Arc::new(p);


        // finish up by grabbing some initialization values for position and size
        let (x,y) = window.get_position().unwrap_or((0,0));
        let (w,h) = window.get_inner_size_pixels().unwrap_or((0,0));
        // TODO: get actual mouse position... or does it matter at this point when we get it in the
        // event loop instead

        let previous_frame_end = Box::new(now(device.clone())) as Box<GpuFuture>;
        let instance = instance.clone();

        Ok(VulkanoRenderer {
            id: game_state::create_next_identity(),
            instance,
            window,
            surface,
            events_loop,
            device,
            queue,
            swapchain,
            images,
            pipeline,
            depth_buffer,
            framebuffers,
            uniform_buffer,
            debug_callback,
            previous_frame_end,
            renderpass: renderpass as Arc<RenderPassAbstract + Send + Sync>,
            recreate_swapchain: false, // flag indicating to rebuild the swapchain on the next frame
            model_data: Vec::new(),
            render_layer_queue: VecDeque::new(),
            fps: fps::FPS::new(),
            current_mouse_pos: ScreenPoint::new(0, 0),
            rect: ScreenRect::new(x as i32, y as i32, w as i32, h as i32),
            debug_world_rotation: 0f32,
            debug_zoom: 0f32,

            // TODO: should DynamicState be reset when the swapchain is rebuilt as well?
            dynamic_state: DynamicState {
                line_width: None,
                viewports: Some(vec![vulkano::pipeline::viewport::Viewport {
                    origin: [0.0, 0.0],
                    dimensions: [dimensions.width() as f32, dimensions.height() as f32],
                    depth_range: 0.0 .. 1.0,
                }]),
                .. DynamicState::none()
            },
        })
    }

    #[inline]
    fn get_mouse_pos(&self) -> &ScreenPoint { &self.current_mouse_pos }

    #[inline]
    fn set_mouse_pos(&mut self, pos: ScreenPoint) { self.current_mouse_pos = pos; }

    #[allow(dead_code)]
    #[inline] fn get_rect(&self) -> &ScreenRect { &self.rect }

    #[inline]
    fn set_rect(&mut self, new_rect: ScreenRect) {
        // TODO: let the renderer know to change things up because we were resized?
        self.flag_recreate_swapchain();

        // TODO: determine a delta here?
        self.rect = new_rect;
    }

    pub fn upload_model(&mut self, model: Arc<game_state::model::Model>) {

        { // save model+material in VulkanoRenderer buffer cache
            let mesh = &model.mesh;
            let vertices: Vec<Vertex> = mesh.vertices.iter().map(|x| Vertex::from(*x)).collect();

            let pixel_buffer = {
                let image = model.material.diffuse_map.to_rgba();
                let image_data = image.into_raw().clone();

                let image_data_chunks = image_data.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]);

                // TODO: staging buffer instead
                vulkano::buffer::cpu_access::CpuAccessibleBuffer::<[[u8; 4]]>
                    ::from_iter(self.device.clone(), BufferUsage::all(), image_data_chunks)
                    .expect("failed to create buffer")
            };

            // TODO: per-model textures are 2048x2048, perhaps this could depend on the image instead?

            let (texture, texture_init) = ImmutableImage::uninitialized(
                self.device.clone(),
                vulkano::image::Dimensions::Dim2d { width: 2048, height: 2048  },
                vulkano::format::R8G8B8A8Unorm,
                MipmapsCount::One,
                ImageUsage {
                    transfer_source: true, // for blits
                    transfer_destination: true,
                    sampled: true,
                    ..ImageUsage::none()
                },
                ImageLayout::ShaderReadOnlyOptimal,
                Some(self.queue.family())
            ).unwrap();

            let texture_init = Arc::new(texture_init);

            let pipeline_set = Self::create_descriptor_set(
                self.device.clone(),
                self.uniform_buffer.clone(),
                self.queue.clone(),
                self.pipeline.clone(),
                0, // we intend this descriptor_set to fit in with the pipeline at set 0
                texture.clone(),
                2048,
                2048
            );

            let item = ModelData {
                model: model.clone(),
                vertices: CpuAccessibleBuffer::from_iter(
                    self.device.clone(), BufferUsage::all(), vertices.iter().cloned()
                ).expect("Unable to create buffer"),
                indices: CpuAccessibleBuffer::from_iter(
                    self.device.clone(), BufferUsage::all(), mesh.indices.iter().cloned()
                ).expect("Unable to create buffer"),
                diffuse_map: pixel_buffer,
                material_data: MaterialRenderData::new(
                    texture.clone(),
                    texture_init.clone(),
                    pipeline_set.clone()
                ),
            };

            // upload to GPU memory
            let cmd_buffer_build = AutoCommandBufferBuilder::primary_one_time_submit(
                self.device.clone(),
                self.queue.family()
            ).unwrap(); // catch oom error here

            let cmd_buffer = cmd_buffer_build.copy_buffer_to_image(
                item.diffuse_map.clone(),
                texture_init.clone()
            ).expect("unable to upload texture").build().expect("unable to build command buffer");

            let prev = mem::replace(
                &mut self.previous_frame_end,
                Box::new(now(self.device.clone())) as Box<GpuFuture>
            );

            let after_future =
                prev.then_execute(self.queue.clone(), cmd_buffer)
                    .expect(
                        &format!("VulkanoRenderer(frame {}, upload_model() ) - unable to execute command buffer", self.fps.count())
                    )
                    .then_signal_fence_and_flush();

            match after_future {
                Ok(future) => {
                    self.model_data.push(item);
                    self.previous_frame_end = Box::new(future) as Box<_>;
                }
                Err(e) => {
                    println!("Error ending frame {:?}", e);
                    self.previous_frame_end = Box::new(vulkano::sync::now(self.device.clone())) as Box<_>;
                }
            }
        }
    }

    fn flag_recreate_swapchain(&mut self) {
        self.recreate_swapchain = true;
    }

    fn render(&mut self, camera: &CameraFacet<f32>) {
        &mut self.previous_frame_end.cleanup_finished();

        if self.recreate_swapchain {
            let size = self.window.get_inner_size_pixels();
            match size {
                Some((w,h)) => {
                    //println!("recreating swapchain with dimensions {:?}", size);
                    use vulkano::swapchain::SwapchainCreationError;

                    let physical = vulkano::instance::PhysicalDevice::enumerate(&self.instance)
                        .next().expect("no device availble");

                    let dims = self.surface.capabilities(physical)
                        .expect("failed to get surface capabilities")
                        .current_extent.unwrap_or([1024,768]);

                    match self.swapchain.recreate_with_dimension(dims) {
                        Ok((new_swapchain, new_images)) => {

                            self.swapchain = new_swapchain;
                            self.images = new_images;

                            self.depth_buffer = AttachmentImage::transient(
                                self.device.clone(),
                                dims,
                                vulkano::format::D16Unorm
                            ).unwrap();

                            self.framebuffers = Self::create_framebuffers(
                                self.renderpass.clone(),
                                self.images.clone(),
                                self.depth_buffer.clone()
                            );

                            self.dynamic_state = DynamicState {
                                line_width: None,
                                viewports: Some(vec![vulkano::pipeline::viewport::Viewport {
                                    origin: [0.0, 0.0],
                                    dimensions: [dims[0] as f32, dims[1] as f32],
                                    depth_range: 0.0 .. 1.0,
                                }]),
                                .. DynamicState::none()
                            };

                            self.recreate_swapchain = false;
                        },
                        Err(SwapchainCreationError::UnsupportedDimensions) => {
                            println!("Unsupported dimensions! {:?}", dims);
                        },
                        Err(e) => panic!("{:?}", e)
                    }
                },
                None => panic!("unable to get dimensions from window")
            }
        }

        // Note the use of 300 micros as a magic number for acquiring a swapchain image
        let (image_num, acquire_future) = match swapchain::acquire_next_image(
            self.swapchain.clone(),
            Some( Duration::from_micros(300) )
        ) {
            Ok((num, future)) => {
                (num, future)
            },
            Err(vulkano::swapchain::AcquireError::OutOfDate) => {
                self.flag_recreate_swapchain();
                return;
            },
            Err(vulkano::swapchain::AcquireError::Timeout) => {
                println!("swapchain::acquire_next_image() Timeout!");
                return;
            },
            Err(e) => panic!("{:?}", e)
        };

        let mut cmd_buffer_build = AutoCommandBufferBuilder::primary_one_time_submit(
            self.device.clone(),
            self.queue.family()
        ).unwrap(); // catch oom error here

        cmd_buffer_build = cmd_buffer_build.begin_render_pass(
            self.framebuffers[image_num].clone(), false,
            vec![
                vulkano::format::ClearValue::from([ 0.0, 0.0, 0.0, 1.0 ]),
                vulkano::format::ClearValue::Depth(1.0)
            ]
        ).expect("unable to begin renderpass");

        let view = {
            camera.view.clone()
        };

        let scale = cgmath::Matrix4::from_scale(1.0);
        let viewscale = view*scale;
        
        // TODO: WIP implement a notion of a camera
        // TODO: do we want to do this every frame?
        let proj_mat = cgmath::perspective(
            cgmath::Rad(::std::f32::consts::FRAC_PI_2),
            {
                let d = ImageAccess::dimensions(&self.images[0]);
                d.width() as f32 / d.height() as f32
            },
            0.01,
            100.0 // depth used for culling!
        );

        {  // modify the data in the uniform buffer for this renderer == our camera
            match self.uniform_buffer.write() {
                Ok(mut write_uniform) => {
                    write_uniform.proj = proj_mat.into();
                }
                Err(err) => println!("Error writing to uniform buffer {:?}", err)
            }
        }


        loop {

            match self.render_layer_queue.pop_front() {
                Some(next_layer) => {

                    // TODO: refactor this to use asset lookups
                    // TODO: refactor this to use WorldEntity collection -> SceneGraph Rc types
                    // TODO: asset lookups should store DescriptorSets with associated textures
                    
                    let iterator = BreadthFirstIterator::new(next_layer.root.clone());
                    for (_node_id, rc) in iterator {
                        let mut node = &mut rc.borrow_mut();

                        // TODO: implement a per model -instance- matrix in the graph itself?
                        let model = self.model_data[node.data as usize].model.clone();

                        let model_mat = model.model_mat.clone();
                        let rotation = cgmath::Matrix4::from_angle_y(
                            cgmath::Rad(self.debug_world_rotation)
                        );
                        let scale = cgmath::Matrix4::from_scale(
                            1.0f32 + self.debug_zoom
                        );
                        let rot_model = model_mat * rotation * scale;

                        // TODO: update the world matrices from the parent * child's local matrix
                        // eg. flag dirty a node, which means all children must be updated
                        // actually save the data in each node
                        let transform_mat = match node.parent() {
                             Some(parent) => {
                                let parent_model_id = parent.borrow().data;
                                let parent_model = &self.model_data[parent_model_id as usize].model;
                                let global_mat = parent_model.world_mat * rot_model;
                                global_mat
                            },
                            None => rot_model
                        };

                        // Push constants are leveraged here to send per-model
                        // matrices into the shaders
                        let push_constants = vs::ty::PushConstants {
                            model_mat:  (viewscale * transform_mat).into(),
                        };

                        let mdl = &self.model_data[node.data as usize];

                        cmd_buffer_build = cmd_buffer_build.draw_indexed(
                                self.pipeline.clone(),
                                self.dynamic_state.clone(),
                                mdl.vertices.clone(),
                                mdl.indices.clone(),
                                mdl.material_data.descriptor_set.clone(),
                                push_constants // or () - both leak on win32...

                        ).expect("Unable to add command");

                    }
                },
                None => break
            }
        }


        let cmd_buffer = cmd_buffer_build.end_render_pass()
                            .expect("unable to end renderpass ")
                            .build()
                            .unwrap();

        let prev = mem::replace(
            &mut self.previous_frame_end,
            Box::new(now(self.device.clone())) as Box<GpuFuture>
        );

        let after_future =
            match prev.join(acquire_future)
                      .then_execute(self.queue.clone(), cmd_buffer) {
                Ok(executed) => {
                    executed.then_swapchain_present(
                        self.queue.clone(),
                        self.swapchain.clone(),
                        image_num
                    ).then_signal_fence_and_flush()
                },
                Err(e) => {
                    self.fps.update();
                    println!(
                        "VulkanoRenderer(frame {}) - unable to execute command buffer, {:?}",
                        self.fps.count(),
                        e
                    );
                    return;
                }
            };

        match after_future {
            Ok(future) => {
                self.previous_frame_end = Box::new(future) as Box<_>;
            }
            Err(vulkano::sync::FlushError::OutOfDate) => {
                println!("swapchain is out of date, flagging recreate_swapchain=true for next frame");

                self.flag_recreate_swapchain();
                self.previous_frame_end = Box::new(vulkano::sync::now(self.device.clone())) as Box<_>;
            }
            Err(e) => {
                println!("Error ending frame {:?}", e);
                self.previous_frame_end = Box::new(vulkano::sync::now(self.device.clone())) as Box<_>;
            }
        }


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

impl Identifyable for VulkanoRenderer {
    fn identify(&self) -> Identity {
        self.id
    }
}

impl Renderer for VulkanoRenderer {
    fn load(&mut self) {
    }

    fn unload(&mut self) {
        self.model_data.clear();
    }

    fn queue_render_layer(&mut self, layer: Arc<SceneGraph>) {
        self.render_layer_queue.push_back(layer);
    }

    fn present(&mut self, camera: &CameraFacet<f32>) {
        self.render(camera);
    }
}

impl Drop for VulkanoRenderer {
    fn drop(&mut self) {
        println!("VulkanRenderer drop");
    }
}

impl InputSource for VulkanoRenderer {
    fn get_input_events(&mut self) -> VecDeque<InputEvent> {

        //println!("get_input_events");
        let mut events = VecDeque::new();
        {
            let event_loop = &mut self.events_loop.lock().unwrap();
            event_loop.poll_events(|e| events.push_back(e.clone()));
        }

        let this_window_id = self.id as u64;
        //test chg

        let mut converted_events = VecDeque::with_capacity(events.len());

        for e in events {

            #[allow(dead_code)]
            match e {
                winit::Event::DeviceEvent{device_id, ref event} => {
                    match event {
                        &winit::DeviceEvent::Added => {},
                        &winit::DeviceEvent::Removed => {},
                        &winit::DeviceEvent::MouseMotion { delta } => {},
                        &winit::DeviceEvent::MouseWheel {delta} => {},
                        &winit::DeviceEvent::Motion { axis, value } => {},
                        &winit::DeviceEvent::Button { button, state } => {},
                        &winit::DeviceEvent::Key(input) => {},
                        &winit::DeviceEvent::Text{codepoint} => {}
                    }
                },
                winit::Event::WindowEvent{ window_id, ref event } => {
                    let maybe_converted_event = match event {
                        // Keyboard Events
                        &winit::WindowEvent::KeyboardInput{device_id, input} => {
                            let e = match input.state {
                                winit::ElementState::Pressed => InputEvent::KeyDown(self.id, input.scancode),
                                winit::ElementState::Released => InputEvent::KeyDown(self.id, input.scancode)
                            };
                            Some(e)
                        }

                        // Mouse Events

                        &winit::WindowEvent::CursorMoved{device_id, position, modifiers} => {
                            let (x,y) = position;
                            let old_pos: ScreenPoint = self.get_mouse_pos().clone();
                            // TODO: resolve f64 truncation to i32 here
                            let new_pos = ScreenPoint::new(x as i32, y as i32);
                            let moved =
                                InputEvent::MouseMove(self.id, new_pos.clone(), DeltaVector::from_points(&old_pos, &new_pos));
                            self.set_mouse_pos(new_pos);
                            Some(moved)
                        },
                        &winit::WindowEvent::MouseInput{device_id, state, button, modifiers} => {
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

                        &winit::WindowEvent::MouseWheel{device_id, delta, phase, modifiers} => {
                            let e = match delta {
                                winit::MouseScrollDelta::LineDelta(x,y) |
                                winit::MouseScrollDelta::PixelDelta(x,y) => {
                                    self.debug_world_rotation += x;
                                    self.debug_zoom += y;
                                    InputEvent::MouseWheel(
                                        self.id, self.get_mouse_pos().clone(),
                                        DeltaVector::new(x as i32, y as i32)
                                    )
                                }
                            };

                            Some(e)
                        },

                        // Window Manager events

                        /*
                        &winit::WindowEvent::MouseEntered => Some(InputEvent::MouseEntered(self.id)),
                        &winit::WindowEvent::MouseLeft => Some(InputEvent::MouseLeft(self.id)),
                        */
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


// DXGI Desktop Duplication API for high-performance screen capture
// This provides a more direct way to capture screen content compared to Windows Graphics Capture API

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_ERROR_INVALID_CALL,
};
use windows::core::Interface;
use super::texture_processor::{TextureProcessor, ProcessedFrame};

#[derive(thiserror::Error, Debug, Clone)]
pub enum DxgiError {
    #[error("Failed to create DXGI factory: {0}")]
    FactoryCreation(String),
    #[error("Failed to create D3D11 device: {0}")]
    DeviceCreation(String),
    #[error("Failed to get DXGI adapter: {0}")]
    AdapterError(String),
    #[error("Failed to get DXGI output: {0}")]
    OutputError(String),
    #[error("Failed to duplicate output: {0}")]
    DuplicationError(String),
    #[error("Access was lost and needs to be re-established")]
    AccessLost,
    #[error("Operation timed out")]
    Timeout,
    #[error("Invalid call to DXGI API")]
    InvalidCall,
    #[error("Windows API error: {0}")]
    WindowsError(#[from] windows::core::Error),
}

pub struct DxgiDesktopDuplication {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    duplication: Option<windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication>,
    texture_processor: TextureProcessor,
}

impl DxgiDesktopDuplication {
    /// Create a new DXGI Desktop Duplication instance
    pub fn new() -> Result<Self, DxgiError> {
        // Create D3D11 device
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| DxgiError::DeviceCreation(e.to_string()))?;
        }
        
        let device = device.ok_or_else(|| DxgiError::DeviceCreation("Device is None".to_string()))?;
        let context = context.ok_or_else(|| DxgiError::DeviceCreation("Context is None".to_string()))?;
        
        // Create texture processor for high-quality frame extraction
        let texture_processor = TextureProcessor::new(device.clone(), context.clone());
        
        Ok(Self {
            device,
            context,
            duplication: None,
            texture_processor,
        })
    }
    
    /// Initialize desktop duplication for the primary monitor
    pub fn initialize_primary_output(&mut self) -> Result<(), DxgiError> {
        unsafe {
            // Create DXGI factory
            let factory: IDXGIFactory1 = CreateDXGIFactory1()
                .map_err(|e| DxgiError::FactoryCreation(e.to_string()))?;
            
            // Get first adapter
            let adapter: IDXGIAdapter1 = factory.EnumAdapters1(0)
                .map_err(|e| DxgiError::AdapterError(e.to_string()))?;
            
            // Get first output (primary monitor)
            let output: IDXGIOutput = adapter.EnumOutputs(0)
                .map_err(|e| DxgiError::OutputError(e.to_string()))?;
            
            // Cast to IDXGIOutput1 for desktop duplication
            let output1: IDXGIOutput1 = output.cast()
                .map_err(|e| DxgiError::OutputError(e.to_string()))?;
            
            // Create desktop duplication
            let duplication = output1.DuplicateOutput(&self.device)
                .map_err(|e| DxgiError::DuplicationError(e.to_string()))?;
            
            self.duplication = Some(duplication);
            
            Ok(())
        }
    }
    
    /// Capture a frame using DXGI Desktop Duplication
    pub fn capture_frame(&mut self) -> Result<Option<ID3D11Texture2D>, DxgiError> {
        let duplication = self.duplication.as_ref()
            .ok_or_else(|| DxgiError::InvalidCall)?;
        
        unsafe {
            let mut frame_info = std::mem::zeroed();
            let mut desktop_resource = None;
            
            match duplication.AcquireNextFrame(0, &mut frame_info, &mut desktop_resource) {
                Ok(_) => {
                    if let Some(resource) = desktop_resource {
                        let texture: ID3D11Texture2D = resource.cast()
                            .map_err(|e| DxgiError::WindowsError(e))?;
                        
                        // Release the frame
                        let _ = duplication.ReleaseFrame();
                        
                        Ok(Some(texture))
                    } else {
                        // Release the frame even if resource is None
                        let _ = duplication.ReleaseFrame();
                        Ok(None)
                    }
                },
                Err(e) => {
                    match e.code() {
                        DXGI_ERROR_WAIT_TIMEOUT => Ok(None), // No new frame
                        DXGI_ERROR_ACCESS_LOST => {
                            self.duplication = None;
                            Err(DxgiError::AccessLost)
                        },
                        DXGI_ERROR_INVALID_CALL => Err(DxgiError::InvalidCall),
                        _ => Err(DxgiError::WindowsError(e)),
                    }
                }
            }
        }
    }
    
    /// Capture and process frame for high-quality minimap detection
    pub fn capture_frame_for_minimap(&mut self) -> Result<Option<ProcessedFrame>, DxgiError> {
        if let Some(texture) = self.capture_frame()? {
            // Use the texture processor to extract frame data
            let processed = self.texture_processor.extract_frame_data(&texture)
                .map_err(|e| DxgiError::DuplicationError(e.to_string()))?;
            Ok(Some(processed))
        } else {
            Ok(None)
        }
    }
    
    /// Extract raw frame data with high quality
    pub fn extract_frame_data(&self, texture: &ID3D11Texture2D) -> Result<ProcessedFrame, DxgiError> {
        self.texture_processor.extract_frame_data(texture)
            .map_err(|e| DxgiError::DuplicationError(e.to_string()))
    }
    
    /// Configure GPU processing
    pub fn set_gpu_processing(&mut self, enabled: bool) {
        self.texture_processor.set_gpu_processing(enabled);
    }
    
    /// Check if duplication is active
    pub fn is_active(&self) -> bool {
        self.duplication.is_some()
    }
    
    /// Reset duplication (useful after access lost)
    pub fn reset(&mut self) {
        self.duplication = None;
    }
}

impl Drop for DxgiDesktopDuplication {
    fn drop(&mut self) {
        // Clean shutdown
        self.duplication = None;
    }
}

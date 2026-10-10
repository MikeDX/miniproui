interface USBDeviceFilter {
  vendorId?: number;
  productId?: number;
  classCode?: number;
  subclassCode?: number;
  protocolCode?: number;
  serialNumber?: string;
}

type USBTransferStatus = "ok" | "stall" | "babble";

interface USBOutTransferResult {
  bytesWritten: number;
  status: USBTransferStatus;
}

interface USBInTransferResult {
  data?: DataView;
  status: USBTransferStatus;
}

interface USBInterface {
  interfaceNumber: number;
  claimed: boolean;
}

interface USBConfiguration {
  configurationValue: number;
  interfaces: USBInterface[];
}

interface USBDevice {
  vendorId: number;
  productId: number;
  opened: boolean;
  configuration: USBConfiguration | null;
  open(): Promise<void>;
  close(): Promise<void>;
  selectConfiguration(configurationValue: number): Promise<void>;
  claimInterface(interfaceNumber: number): Promise<void>;
  transferIn(endpointNumber: number, length: number): Promise<USBInTransferResult>;
  transferOut(endpointNumber: number, data: BufferSource): Promise<USBOutTransferResult>;
}

interface USB {
  requestDevice(options?: { filters?: USBDeviceFilter[] }): Promise<USBDevice>;
}

interface Navigator {
  usb?: USB;
}

interface FilePickerAcceptType {
  description?: string;
  accept: Record<string, string | string[]>;
}

interface OpenFilePickerOptions {
  multiple?: boolean;
  excludeAcceptAllOption?: boolean;
  types?: FilePickerAcceptType[];
}

interface SaveFilePickerOptions {
  suggestedName?: string;
  excludeAcceptAllOption?: boolean;
  types?: FilePickerAcceptType[];
}

interface Window {
  showOpenFilePicker(options?: OpenFilePickerOptions): Promise<FileSystemFileHandle[]>;
  showSaveFilePicker(options?: SaveFilePickerOptions): Promise<FileSystemFileHandle>;
}

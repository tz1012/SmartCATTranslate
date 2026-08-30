#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <PDFKit/PDFKit.h>
#include <stdint.h>
#include <stdlib.h>

int smartcat_render_pdf_page(const char *utf8_path, uint32_t page_index, uint32_t dpi,
                             uint8_t **out_bytes, uint32_t *out_width,
                             uint32_t *out_height, size_t *out_length) {
  if (!utf8_path || !out_bytes || !out_width || !out_height || !out_length) return 1;
  @autoreleasepool {
    NSString *path = [NSString stringWithUTF8String:utf8_path];
    PDFDocument *document = [[PDFDocument alloc] initWithURL:[NSURL fileURLWithPath:path]];
    if (!document || document.isLocked || page_index >= document.pageCount) return 2;
    PDFPage *page = [document pageAtIndex:page_index];
    NSRect box = [page boundsForBox:kPDFDisplayBoxCropBox];
    CGFloat scale = MIN(MAX(dpi, 72), 300) / 72.0;
    uint32_t width = (uint32_t)ceil(box.size.width * scale);
    uint32_t height = (uint32_t)ceil(box.size.height * scale);
    if (!width || !height || width > 8192 || height > 8192 || ((uint64_t)width * height) > 80000000) return 3;
    size_t length = (size_t)width * height * 4;
    uint8_t *pixels = calloc(1, length); if (!pixels) return 4;
    CGColorSpaceRef color = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(pixels, width, height, 8, width * 4, color, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(color);
    if (!context) { free(pixels); return 5; }
    CGContextSetRGBFillColor(context, 1, 1, 1, 1); CGContextFillRect(context, CGRectMake(0, 0, width, height));
    CGContextScaleCTM(context, scale, scale); [page drawWithBox:kPDFDisplayBoxCropBox toContext:context]; CGContextRelease(context);
    *out_bytes = pixels; *out_width = width; *out_height = height; *out_length = length; return 0;
  }
}
void smartcat_free_pdf_page(uint8_t *bytes) { free(bytes); }

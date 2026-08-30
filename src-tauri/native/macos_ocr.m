#import <Foundation/Foundation.h>
#import <Vision/Vision.h>
#import <CoreGraphics/CoreGraphics.h>

static void set_error(char **error_code, NSString *value) {
  if (error_code) *error_code = strdup(value.UTF8String);
}

int smartcat_vision_ocr(const unsigned char *rgba, int width, int height,
                        const char *hints_json, char **output_json,
                        char **error_code) {
  @autoreleasepool {
    if (@available(macOS 10.15, *)) {
      if (!rgba || width <= 0 || height <= 0 || !output_json) {
        set_error(error_code, @"invalid_result"); return 0;
      }
      NSData *pixels = [NSData dataWithBytes:rgba length:(NSUInteger)width * height * 4];
      CGDataProviderRef provider = CGDataProviderCreateWithCFData((__bridge CFDataRef)pixels);
      CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
      CGImageRef image = CGImageCreate(width, height, 8, 32, width * 4, colorSpace,
        kCGBitmapByteOrder32Big | kCGImageAlphaLast, provider, NULL, false,
        kCGRenderingIntentDefault);
      CGColorSpaceRelease(colorSpace); CGDataProviderRelease(provider);
      if (!image) { set_error(error_code, @"invalid_result"); return 0; }
      __block NSArray<VNRecognizedTextObservation *> *observations = nil;
      __block NSError *requestError = nil;
      VNRecognizeTextRequest *request = [[VNRecognizeTextRequest alloc]
        initWithCompletionHandler:^(VNRequest *completed, NSError *error) {
          requestError = error;
          observations = (NSArray<VNRecognizedTextObservation *> *)completed.results;
        }];
      request.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
      request.usesLanguageCorrection = YES;
      if (hints_json) {
        NSData *hintData = [[NSString stringWithUTF8String:hints_json] dataUsingEncoding:NSUTF8StringEncoding];
        NSArray *hints = hintData ? [NSJSONSerialization JSONObjectWithData:hintData options:0 error:nil] : nil;
        if ([hints isKindOfClass:NSArray.class] && hints.count) request.recognitionLanguages = hints;
      }
      VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
      NSError *performError = nil;
      BOOL ok = [handler performRequests:@[request] error:&performError];
      CGImageRelease(image);
      if (!ok || requestError || performError) {
        NSError *error = requestError ?: performError;
        set_error(error_code, error.code == 18 ? @"language_pack_missing" : @"native_failure");
        return 0;
      }
      NSMutableArray *result = [NSMutableArray arrayWithCapacity:observations.count];
      for (VNRecognizedTextObservation *observation in observations) {
        VNRecognizedText *candidate = [observation topCandidates:1].firstObject;
        if (!candidate) continue;
        CGRect box = observation.boundingBox;
        [result addObject:@{@"text": candidate.string ?: @"", @"x": @(box.origin.x * width),
          @"y": @((1.0 - box.origin.y - box.size.height) * height), @"width": @(box.size.width * width),
          @"height": @(box.size.height * height), @"confidence": @(candidate.confidence), @"angleDegrees": @0.0}];
      }
      NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
      if (!json) { set_error(error_code, @"invalid_result"); return 0; }
      NSString *string = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
      *output_json = strdup(string.UTF8String); return 1;
    }
    set_error(error_code, @"unsupported_os_version"); return 0;
  }
}

void smartcat_free_string(char *value) { if (value) free(value); }

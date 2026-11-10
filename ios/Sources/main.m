#import <UIKit/UIKit.h>

extern void droplink_ios_start(void);

int main(int argc, char *argv[]) {
    @autoreleasepool {
        droplink_ios_start();
    }
    return 0;
}

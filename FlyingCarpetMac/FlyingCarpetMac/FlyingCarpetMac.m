//
//  FlyingCarpetMac.m
//  FlyingCarpetMac
//
//  Created by Theron on 12/27/22.
//

#import "FlyingCarpetMac.h"

@implementation FlyingCarpetMac
@end

#import <Foundation/Foundation.h>
#import <CoreWLAN/CoreWLAN.h>
#import <SecurityFoundation/SFAuthorization.h>

SFAuthorization *auth = nil;

uint8_t joinAdHoc(char * cSSID, char * cPassword) {
    NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
    NSString * password = [[NSString alloc] initWithUTF8String:cPassword];
    CWInterface * iface = CWWiFiClient.sharedWiFiClient.interface;
    NSError * ibssErr = nil;
    NSSet<CWNetwork *> * network = [iface scanForNetworksWithName:SSID error:&ibssErr];
    BOOL result = [iface associateToNetwork:network.anyObject password:password error:&ibssErr];
    if (!result) {
        NSLog(@"joinAdHoc error: %@", ibssErr.localizedDescription);
    }
    return result;
}

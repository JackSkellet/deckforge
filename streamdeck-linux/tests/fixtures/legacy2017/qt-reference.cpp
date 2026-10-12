// Our synthetic fixture generator. It does not load or execute Elgato code.
// The matrices and serialization layout are independently transcribed from
// the pinned native instructions; Qt supplies generic QImage transform semantics.
#include <QImage>
#include <QTransform>
#include <algorithm>
#include <fstream>
#include <vector>
int main(int argc,char**argv){
 if(argc!=2)return 2;
 QImage image(72,72,QImage::Format_RGB888);
 for(int y=0;y<72;y++)for(int x=0;x<72;x++)image.setPixelColor(x,y,x<36?(y<36?Qt::red:Qt::blue):(y<36?Qt::green:Qt::yellow));
 for(int vertical:{-1,1}){
  QImage transformed=image.transformed(QTransform(-1,0,0,vertical,0,0),Qt::FastTransformation).convertToFormat(QImage::Format_BGR888);
  std::vector<unsigned char> bmp(15606,0);
  auto u32=[&](int at,unsigned v){for(int i=0;i<4;i++)bmp[at+i]=(v>>(i*8))&255;};
  bmp[0]='B';bmp[1]='M';u32(2,15606);u32(10,54);u32(14,40);u32(18,72);u32(22,72);bmp[26]=1;bmp[28]=24;
  for(int y=0;y<72;y++)std::copy_n(transformed.constScanLine(71-y),216,bmp.data()+54+y*216);
  std::ofstream f(std::string(argv[1])+(vertical<0?"/rotate180.bmp":"/flip-horizontal.bmp"),std::ios::binary);
  f.write(reinterpret_cast<char*>(bmp.data()),bmp.size());
 }
}

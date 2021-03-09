require 'pathname'
require 'digest/md5'

def stub_files(path)
    sorted_list_of_files_under(path)
end


def sorted_list_of_files_under(dir)
    result = []
    base_path = Pathname(dir)
    Dir.chdir(base_path) do
        Pathname('.').find do |path|
        result << path unless path.to_s == '.'
        end
    end
    result.sort
end

base_path = Pathname(ARGV[0])
digest = Digest::MD5.new
Dir.chdir(base_path) do
    stub_files(base_path).each do |path|
        puts "updating " + path.to_s
        digest.update(path.to_s)
        puts "updating with file" unless path.directory?
        digest.file(path.to_s) unless path.directory?
        puts ""
    end
end
checksum = digest.hexdigest
puts checksum

